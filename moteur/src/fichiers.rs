//! Reconnaît le type d'un fichier et prépare ce qu'on envoie au modèle.

use crate::documents::{self, Bloc};
use crate::ocr::{Ocr, texte_utile};
use crate::photo::{Images, Pixels};
use crate::video;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pdfium_render::prelude::*;
/// Pour garder pdfium chargée ailleurs (l'aperçu des pages).
pub use pdfium_render::prelude::Pdfium;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

/// Côté le plus long envoyé au modèle : il découpe ensuite en 280 jetons au plus.
const COTE_MAX: u32 = 768;
/// En dessous (plus grand côté en pixels), une image est une icône : pas de lecture du contenu.
const IMAGE_COTE_MIN: u32 = 200;
/// Côté le plus long pour l'OCR : assez grand pour lire les petits textes.
const COTE_OCR: u32 = 2400;
const PASSAGE_CARACTERES: usize = 1000;
const PASSAGE_RECOUVREMENT_MOTS: usize = 25;
const TRANCHE_SECONDES: usize = 30;
const FIN_MIN_SECONDES: usize = 10;
const ECHANTILLONS_PAR_SECONDE: usize = 16_000;
/// Jetons d'une image (--image-max-tokens) et d'une seconde de son (fiche de Google).
pub const JETONS_IMAGE: usize = 280;
const JETONS_SON_PAR_SECONDE: usize = 25;
/// Dans un fichier de texte, les premiers passages reçoivent tous un vecteur (≈ 40 000 caractères)…
const PASSAGES_PLEINS: usize = 40;
/// … puis un sur quatre…
const PASSAGES_PAS: usize = 4;
/// … et jamais plus que ça. Les autres restent trouvables par leurs mots exacts.
/// Mesuré sur l'index de l'utilisateur (11 novembre 2026) : 1,6 % des fichiers de texte portaient 51 % des
/// 236 000 passages, et le modèle fait les trois quarts du temps d'indexation.
const PASSAGES_MAX: usize = 200;
/// Dans une archive : au plus ce nombre de fichiers lus, ce total, et pas de fichier plus gros que ça.
const ARCHIVE_FICHIERS_MAX: usize = 300;
const ARCHIVE_OCTETS_MAX: u64 = 300 << 20;
const ARCHIVE_FICHIER_OCTETS_MAX: u64 = 50 << 20;
/// Passages de texte d'une archive qui reçoivent un vecteur, tous fichiers confondus (les suivants : mots exacts).
const ARCHIVE_PASSAGES_MAX: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Genre {
    Photo,
    Pdf,
    Audio,
    Video,
    Document,
    Tableur,
    Texte,
    Code,
    /// Une archive .zip : les fichiers qu'elle contient sont lus sans l'ouvrir.
    Archive,
}

const GENRES: [Genre; 9] = [
    Genre::Photo,
    Genre::Pdf,
    Genre::Audio,
    Genre::Video,
    Genre::Document,
    Genre::Tableur,
    Genre::Texte,
    Genre::Code,
    Genre::Archive,
];

impl Genre {
    pub fn nom(self) -> &'static str {
        match self {
            Genre::Photo => "photo",
            Genre::Pdf => "pdf",
            Genre::Audio => "audio",
            Genre::Video => "video",
            Genre::Document => "document",
            Genre::Tableur => "tableur",
            Genre::Texte => "texte",
            Genre::Code => "code",
            Genre::Archive => "archive",
        }
    }

    pub fn noms() -> String {
        GENRES.map(|g| g.nom()).join(", ")
    }

    pub fn depuis_nom(nom: &str) -> Option<Self> {
        GENRES.into_iter().find(|g| g.nom() == nom)
    }

    pub fn de(chemin: &Path) -> Option<Self> {
        let ext = chemin.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "jxr"
            | "dng" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "raf" => Genre::Photo,
            "pdf" => Genre::Pdf,
            "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wma" | "aiff" | "aif" | "amr" => Genre::Audio,
            "mp4" | "m4v" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "mpg" | "mpeg" | "mts" | "m2ts" | "3gp"
            | "flv" => Genre::Video,
            "docx" | "docm" | "pptx" | "pptm" | "odt" | "odp" => Genre::Document,
            "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" | "csv" | "tsv" => Genre::Tableur,
            "txt" | "md" | "markdown" | "rst" | "log" | "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg"
            | "html" | "htm" | "eml" | "srt" | "vtt" | "tex" => Genre::Texte,
            "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "java" | "c" | "h" | "cpp" | "cc" | "hpp"
            | "cs" | "go" | "rb" | "php" | "swift" | "kt" | "kts" | "scala" | "sh" | "ps1" | "bat" | "cmd"
            | "sql" | "lua" | "r" | "dart" | "vue" | "svelte" | "css" | "scss" | "less" => Genre::Code,
            "zip" => Genre::Archive,
            _ => return None,
        })
    }
}

/// Un morceau de fichier qui deviendra un vecteur.
pub struct Morceau {
    /// image, texte_image, page, page_ocr, son, plan, texte, diapo, feuille, ligne
    pub nature: &'static str,
    /// Page, diapo, feuille ou ligne (à partir de 1) ; seconde de début (son, plan) ; 0 sinon.
    pub debut: f64,
    /// Seconde de fin (son, plan).
    pub fin: Option<f64>,
    /// Ce qu'on envoie au modèle. Nul : pas de vecteur pour ce morceau, seulement son texte (mots exacts).
    pub entree: Value,
    /// Estimation du nombre de jetons, pour remplir les lots sans dépasser le contexte.
    pub poids: usize,
    /// Le passage de texte (documents, OCR) : gardé pour les mots exacts et les extraits.
    pub texte: Option<String>,
    /// Dans une archive : le chemin du fichier dans l'archive.
    pub partie: Option<String>,
}

#[derive(Default)]
pub struct Preparation {
    pub morceaux: Vec<Morceau>,
    /// Pages sans texte intégré (scans).
    pub pages_sans_texte: usize,
    pub ocr_ms: f64,
}

/// pdfium.dll, chargée une seule fois par programme (pdfium-render refuse un deuxième chargement : les
/// sessions d'indexation suivantes échouaient) et partagée entre les fils (pdfium-render la protège par un verrou).
pub fn charger_pdfium(racine: &Path) -> Result<Arc<Pdfium>> {
    static PDFIUM: std::sync::OnceLock<Arc<Pdfium>> = std::sync::OnceLock::new();
    if let Some(p) = PDFIUM.get() {
        return Ok(p.clone());
    }
    let dll = Pdfium::pdfium_platform_library_name_at_path(&racine.join("outils").join("pdfium").join("bin"));
    let pdfium = Arc::new(Pdfium::new(Pdfium::bind_to_library(&dll).context("chargement de pdfium.dll")?));
    Ok(PDFIUM.get_or_init(|| pdfium).clone())
}

/// Les lecteurs d'un fil de préparation (WIC et l'OCR ne se partagent pas entre fils).
pub struct Lecteurs {
    images: Images,
    pdfium: Arc<Pdfium>,
    pub ocr: Option<Ocr>,
}

impl Lecteurs {
    pub fn nouveaux(pdfium: Arc<Pdfium>, avec_ocr: bool) -> Result<Self> {
        let images = Images::nouveau()?;
        let ocr = if avec_ocr {
            match Ocr::nouveau() {
                Ok(o) => Some(o),
                Err(e) => {
                    eprintln!("OCR désactivé : {e:#}");
                    None
                }
            }
        } else {
            None
        };
        Ok(Lecteurs { images, pdfium, ocr })
    }

    pub fn preparer(&self, chemin: &Path, genre: Genre) -> Result<Preparation> {
        let mut prep = self.lire(chemin, genre)?;
        // Texte, code, tableurs : les fichiers géants sont presque toujours faits par des machines (code
        // empaqueté, journaux, exports). Les PDF et les documents, écrits pour être lus, restent entiers.
        if matches!(genre, Genre::Texte | Genre::Code | Genre::Tableur) {
            alleger(&mut prep.morceaux);
        }
        Ok(prep)
    }

    fn lire(&self, chemin: &Path, genre: Genre) -> Result<Preparation> {
        let titre = chemin.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match genre {
            Genre::Photo => self.photo(chemin, &titre),
            Genre::Pdf => self.pdf(chemin, &titre),
            Genre::Audio => audio(chemin),
            Genre::Video => video_complete(chemin),
            Genre::Document => Ok(blocs_en_morceaux(&titre, documents::office(chemin, &ext)?)),
            Genre::Tableur if ext == "csv" || ext == "tsv" => Ok(blocs_en_morceaux(&titre, documents::texte(chemin, false)?)),
            Genre::Tableur => Ok(blocs_en_morceaux(&titre, documents::tableur(chemin)?)),
            Genre::Texte | Genre::Code => {
                Ok(blocs_en_morceaux(&titre, documents::texte(chemin, ext == "html" || ext == "htm")?))
            }
            Genre::Archive => self.archive(chemin),
        }
    }

    /// Les fichiers d'une archive : chacun est copié à part un instant et lu comme s'il était sur le disque
    /// (photos, PDF, documents, textes, code ; pas les sons ni les vidéos, trop longs, ni les archives dedans).
    fn archive(&self, chemin: &Path) -> Result<Preparation> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(chemin)?).context("archive illisible")?;
        let mut prep = Preparation::default();
        // Un logiciel (installateur, paquet) : trouvable par son nom, pas par ses traductions ni ses images.
        if archive_de_logiciel(zip.file_names().filter_map(|n| n.ok())) {
            return Ok(prep);
        }
        let dossier = DossierTemporaire::nouveau()?;
        let (mut lus, mut octets) = (0, 0u64);
        let mut passages = 0;
        for i in 0..zip.len() {
            if lus >= ARCHIVE_FICHIERS_MAX || octets >= ARCHIVE_OCTETS_MAX {
                break;
            }
            let Ok(mut entree) = zip.by_index(i) else { continue };
            // `enclosed_name` refuse les chemins qui sortiraient du dossier (« ../../ »).
            let Some(interne) = entree.enclosed_name() else { continue };
            if !de_personne_dans_archive(&interne) {
                continue;
            }
            let Some(genre) = Genre::de(&interne) else { continue };
            if entree.is_dir() || entree.size() > ARCHIVE_FICHIER_OCTETS_MAX {
                continue;
            }
            let Some(nom) = interne.file_name() else { continue };
            // Un sous-dossier par fichier : deux « notes.txt » de l'archive ne se mélangent pas.
            let copie = dossier.chemin.join(i.to_string()).join(nom);
            let copie_faite = std::fs::create_dir_all(copie.parent().unwrap_or(&dossier.chemin))
                .and_then(|_| std::fs::File::create(&copie))
                .and_then(|mut f| std::io::copy(&mut entree, &mut f));
            let Ok(taille) = copie_faite else { continue };
            octets += taille;
            lus += 1;
            if let Ok(p) = self.preparer(&copie, genre) {
                let partie = interne.to_string_lossy().replace('\\', "/");
                prep.pages_sans_texte += p.pages_sans_texte;
                prep.ocr_ms += p.ocr_ms;
                for mut m in p.morceaux {
                    if m.texte.is_some() && !m.entree.is_null() {
                        passages += 1;
                        if passages > ARCHIVE_PASSAGES_MAX {
                            m.entree = Value::Null;
                            m.poids = 0;
                        }
                    }
                    prep.morceaux.push(Morceau { partie: Some(partie.clone()), ..m });
                }
            }
            let _ = std::fs::remove_file(&copie);
        }
        Ok(prep)
    }

    fn photo(&self, chemin: &Path, titre: &str) -> Result<Preparation> {
        let source = self.images.ouvrir(chemin).context("décodage WIC")?;
        let mut prep = Preparation::default();
        // Icône, vignette, élément d'interface : retrouvable par son nom seulement.
        if source.largeur.max(source.hauteur) < IMAGE_COTE_MIN {
            return Ok(prep);
        }
        let jpeg = self.images.jpeg(&source, COTE_MAX)?;
        prep.morceaux.push(images_en_morceau("image", 0.0, None, &[jpeg]));
        if let Some(ocr) = &self.ocr {
            let t = Instant::now();
            let texte = self.images.pixels(&source, COTE_OCR.min(ocr.cote_max), true).and_then(|p| ocr.lire(&p));
            prep.ocr_ms = t.elapsed().as_secs_f64() * 1000.0;
            if let Ok(texte) = texte {
                if texte_utile(&texte) {
                    ajouter_texte(&mut prep.morceaux, titre, &texte, "texte_image", 0.0);
                }
            }
        }
        Ok(prep)
    }

    fn pdf(&self, chemin: &Path, titre: &str) -> Result<Preparation> {
        let document = self.pdfium.load_pdf_from_file(chemin, None)?;
        let mut prep = Preparation::default();
        for (i, page) in document.pages().iter().enumerate() {
            let numero = (i + 1) as f64;
            let texte = page.text()?.all();
            if !decouper(&texte).is_empty() {
                ajouter_texte(&mut prep.morceaux, titre, &texte, "page", numero);
                continue;
            }
            prep.pages_sans_texte += 1;
            let Some(ocr) = &self.ocr else { continue };
            let t = Instant::now();
            let config = PdfRenderConfig::new()
                .set_target_width(COTE_OCR.min(ocr.cote_max) as i32)
                .set_format(PdfBitmapFormat::BGRA);
            let lu = page.render_with_config(&config).map_err(anyhow::Error::from).and_then(|rendu| {
                ocr.lire(&Pixels {
                    largeur: rendu.width() as u32,
                    hauteur: rendu.height() as u32,
                    octets: rendu.as_raw_bytes(),
                })
            });
            prep.ocr_ms += t.elapsed().as_secs_f64() * 1000.0;
            if let Ok(texte) = lu {
                if texte_utile(&texte) {
                    ajouter_texte(&mut prep.morceaux, titre, &texte, "page_ocr", numero);
                }
            }
        }
        Ok(prep)
    }
}

/// Retire l'entrée du modèle (`entree` nulle) des passages en trop d'un très gros fichier : ils gardent
/// leur texte, donc restent trouvables par leurs mots exacts, mais ne coûtent plus rien au modèle.
fn alleger(morceaux: &mut [Morceau]) {
    let (mut rang, mut gardes) = (0, 0);
    for m in morceaux.iter_mut().filter(|m| m.texte.is_some()) {
        rang += 1;
        let garder = rang <= PASSAGES_PLEINS || ((rang - PASSAGES_PLEINS) % PASSAGES_PAS == 0 && gardes < PASSAGES_MAX);
        if garder {
            gardes += 1;
        } else {
            m.entree = Value::Null;
            m.poids = 0;
        }
    }
}

/// Dans une archive, ce qu'une personne y range : photos, PDF, documents, tableurs, notes. Pas le code, les
/// réglages ni les pages web : une archive qui en contient est presque toujours un logiciel (installateur,
/// paquet), dont les traductions et les licences noyaient les vrais résultats.
pub fn de_personne_dans_archive(interne: &Path) -> bool {
    let ext = interne.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match Genre::de(interne) {
        Some(Genre::Photo | Genre::Pdf | Genre::Document | Genre::Tableur) => true,
        Some(Genre::Texte) => matches!(ext.as_str(), "txt" | "md" | "markdown" | "rst" | "eml"),
        _ => false,
    }
}

/// Une archive qui contient des programmes, ou surtout du code et des réglages : un logiciel, pas des
/// affaires rangées par quelqu'un.
pub fn archive_de_logiciel<S: AsRef<str>>(noms: impl Iterator<Item = S>) -> bool {
    let (mut fichiers, mut techniques) = (0, 0);
    for nom in noms {
        let nom = nom.as_ref();
        if nom.ends_with('/') {
            continue;
        }
        fichiers += 1;
        let ext = Path::new(nom).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "exe" | "dll" | "msi" | "sys" | "so" | "dylib" | "jar" | "class" | "pyd" | "node" | "apk") {
            return true;
        }
        let technique = matches!(Genre::de(Path::new(nom)), Some(Genre::Code))
            || matches!(ext.as_str(), "json" | "xml" | "html" | "htm" | "ini" | "cfg" | "yaml" | "yml" | "toml" | "plist" | "dat" | "bin" | "pak");
        techniques += usize::from(technique);
    }
    fichiers > 0 && techniques * 5 > fichiers
}

/// Un dossier temporaire à soi, effacé quand on n'en a plus besoin (même après une erreur).
struct DossierTemporaire {
    chemin: std::path::PathBuf,
}

impl DossierTemporaire {
    fn nouveau() -> Result<Self> {
        static NUMERO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NUMERO.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let chemin = std::env::temp_dir().join(format!("furet-archive-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&chemin)?;
        Ok(DossierTemporaire { chemin })
    }
}

impl Drop for DossierTemporaire {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.chemin);
    }
}

/// Les noms des fichiers d'une archive (pour l'aperçu), et leur nombre.
pub fn contenu_archive(chemin: &Path, max: usize) -> Result<(Vec<String>, usize)> {
    let zip = zip::ZipArchive::new(std::fs::File::open(chemin)?).context("archive illisible")?;
    let total = zip.len();
    let mut noms: Vec<String> =
        zip.file_names().filter_map(|n| n.ok()).filter(|n| !n.ends_with('/')).map(String::from).collect();
    noms.sort_by_key(|n| n.to_lowercase());
    noms.truncate(max);
    Ok((noms, total))
}

/// Le texte d'un document, d'un tableur, d'un texte ou de code, par blocs (pour l'aperçu).
pub fn texte_pour_apercu(chemin: &Path) -> Result<Vec<Bloc>> {
    let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match Genre::de(chemin) {
        Some(Genre::Document) => documents::office(chemin, &ext),
        Some(Genre::Tableur) if ext == "csv" || ext == "tsv" => documents::texte(chemin, false),
        Some(Genre::Tableur) => documents::tableur(chemin),
        Some(Genre::Texte | Genre::Code) => documents::texte(chemin, ext == "html" || ext == "htm"),
        _ => bail!("pas de texte à montrer"),
    }
}

pub fn nombre_de_pages(pdfium: &Pdfium, chemin: &Path) -> Result<u32> {
    Ok(pdfium.load_pdf_from_file(chemin, None)?.pages().len() as u32)
}

/// La page `numero` (à partir de 1) d'un PDF en JPEG, `largeur` pixels de large, et le nombre de pages.
pub fn rendre_page(pdfium: &Pdfium, chemin: &Path, numero: u32, largeur: u32) -> Result<(Vec<u8>, u32)> {
    let document = pdfium.load_pdf_from_file(chemin, None)?;
    let pages = document.pages();
    let total = pages.len() as u32;
    let page = pages.get((numero.clamp(1, total.max(1)) as u16 - 1).into())?;
    let config = PdfRenderConfig::new().set_target_width(largeur as i32).set_format(PdfBitmapFormat::BGRA);
    let rendu = page.render_with_config(&config)?;
    let (l, h) = (rendu.width() as u32, rendu.height() as u32);
    // BGRA → BGR, ce qu'attend l'encodeur.
    let octets: Vec<u8> = rendu.as_raw_bytes().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let jpeg = crate::photo::encoder_jpeg(&Pixels { largeur: l, hauteur: h, octets })?;
    Ok((jpeg, total))
}

fn images_en_morceau(nature: &'static str, debut: f64, fin: Option<f64>, jpegs: &[Vec<u8>]) -> Morceau {
    let parties: Vec<Value> = jpegs
        .iter()
        .map(|j| json!({ "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", STANDARD.encode(j)) } }))
        .collect();
    Morceau {
        nature,
        debut,
        fin,
        entree: json!({ "content": parties }),
        poids: JETONS_IMAGE * jpegs.len(),
        texte: None,
        partie: None,
    }
}

/// Ajoute un texte découpé en passages, avec le préfixe « document » de EmbeddingGemma.
fn ajouter_texte(morceaux: &mut Vec<Morceau>, titre: &str, texte: &str, nature: &'static str, debut: f64) {
    for p in decouper(texte) {
        let entree = format!("title: {titre} | text: {p}");
        morceaux.push(Morceau {
            nature,
            debut,
            fin: None,
            poids: entree.len() / 3 + 8,
            entree: Value::String(entree),
            texte: Some(p),
            partie: None,
        });
    }
}

fn blocs_en_morceaux(titre: &str, blocs: Vec<Bloc>) -> Preparation {
    let mut prep = Preparation::default();
    for b in blocs {
        ajouter_texte(&mut prep.morceaux, titre, &b.texte, b.nature, b.numero);
    }
    prep
}

/// Découpe un texte en passages d'environ PASSAGE_CARACTERES, avec un petit recouvrement.
fn decouper(texte: &str) -> Vec<String> {
    let mots: Vec<&str> = texte.split_whitespace().collect();
    if mots.iter().map(|m| m.len()).sum::<usize>() < 20 {
        return Vec::new();
    }
    let mut passages = Vec::new();
    let mut debut = 0;
    while debut < mots.len() {
        let mut fin = debut;
        let mut longueur = 0;
        while fin < mots.len() && (longueur == 0 || longueur + mots[fin].len() < PASSAGE_CARACTERES) {
            longueur += mots[fin].len() + 1;
            fin += 1;
        }
        // Un « mot » géant (base64, code minifié) est coupé pour rester dans la taille d'un passage.
        let passage = mots[debut..fin].join(" ");
        passages.push(passage.chars().take(PASSAGE_CARACTERES * 2).collect());
        if fin == mots.len() {
            break;
        }
        debut = fin.saturating_sub(PASSAGE_RECOUVREMENT_MOTS).max(debut + 1);
    }
    passages
}

/// Plans de la vidéo, puis sa piste son (si elle en a une).
fn video_complete(chemin: &Path) -> Result<Preparation> {
    let mut prep = Preparation::default();
    for plan in video::plans(chemin)? {
        prep.morceaux.push(images_en_morceau("plan", plan.debut, Some(plan.fin), &plan.images));
    }
    if let Ok(son) = audio(chemin) {
        prep.morceaux.extend(son.morceaux);
    }
    if prep.morceaux.is_empty() {
        bail!("aucune image ni son lisible");
    }
    Ok(prep)
}

/// Le son, par tranches de 30 s. La parole est transcrite plus tard, dans une deuxième passe.
fn audio(chemin: &Path) -> Result<Preparation> {
    let mut prep = Preparation::default();
    for (debut, fin, tranche) in tranches_audio(chemin)? {
        prep.morceaux.push(Morceau {
            nature: "son",
            debut,
            fin: Some(fin),
            entree: json!({ "content": [{ "type": "input_audio",
                "input_audio": { "data": STANDARD.encode(wav(&tranche)), "format": "wav" } }] }),
            poids: (fin - debut).ceil() as usize * JETONS_SON_PAR_SECONDE,
            texte: None,
            partie: None,
        });
    }
    Ok(prep)
}

/// Décode l'audio en mono 16 kHz avec ffmpeg et le découpe en tranches de 30 s (début, fin, échantillons),
/// en sautant les silences.
pub fn tranches_audio(chemin: &Path) -> Result<Vec<(f64, f64, Vec<i16>)>> {
    let sortie = crate::processus::commande_basse(crate::processus::ffmpeg("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(chemin)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .output()
        .context("lancement de ffmpeg (est-il dans le PATH ?)")?;
    if !sortie.status.success() {
        bail!("ffmpeg n'a pas pu lire le son");
    }
    let echantillons: Vec<i16> = sortie.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let par_tranche = TRANCHE_SECONDES * ECHANTILLONS_PAR_SECONDE;
    let mut tranches = Vec::new();
    let mut a = 0;
    while a < echantillons.len() {
        let mut b = (a + par_tranche).min(echantillons.len());
        // Une fin de moins de 10 s rejoint la tranche d'avant : seule, elle donne un vecteur peu fiable.
        if echantillons.len() - b < FIN_MIN_SECONDES * ECHANTILLONS_PAR_SECONDE {
            b = echantillons.len();
        }
        let tranche = &echantillons[a..b];
        let debut = a as f64 / ECHANTILLONS_PAR_SECONDE as f64;
        let fin = b as f64 / ECHANTILLONS_PAR_SECONDE as f64;
        a = b;
        if tranche.len() < ECHANTILLONS_PAR_SECONDE || silencieux(tranche) {
            continue;
        }
        tranches.push((debut, fin, tranche.to_vec()));
    }
    Ok(tranches)
}

/// Moins de -46 dB en moyenne : on considère la tranche comme du silence.
fn silencieux(tranche: &[i16]) -> bool {
    let carres: f64 = tranche.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
    (carres / tranche.len() as f64).sqrt() < 0.005
}

fn wav(echantillons: &[i16]) -> Vec<u8> {
    let donnees = (echantillons.len() * 2) as u32;
    let taux = ECHANTILLONS_PAR_SECONDE as u32;
    let mut w = Vec::with_capacity(44 + donnees as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + donnees).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&taux.to_le_bytes());
    w.extend_from_slice(&(taux * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&donnees.to_le_bytes());
    for s in echantillons {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}
