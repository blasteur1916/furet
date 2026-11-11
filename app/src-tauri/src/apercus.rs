//! L'aperçu d'un résultat (Espace) : la photo, la page du PDF, le passage de la vidéo ou du son, le texte
//! autour de ce qui a été trouvé, ce que contient un dossier. Les fichiers sont servis à l'interface par
//! `http://furet-fichier.localhost/<clé>` (seulement ceux que la recherche a proposés) : en entier ou par
//! plages pour les vidéos et les sons, `/<clé>/image` pour une photo réduite, `/<clé>/page/<n>` pour une page.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use furet_moteur::fichiers::{self, Genre, Pdfium};
use furet_moteur::filtres::{date_lisible, taille_lisible};
use furet_moteur::photo::Images;
use serde::Serialize;
use tauri::http::{Request, Response, StatusCode, header};

use crate::{miniatures, recherche};

pub const SCHEMA: &str = "furet-fichier";
/// Une vidéo ou un son est envoyé par morceaux de cette taille (le lecteur demande la suite).
const MORCEAU: u64 = 4 << 20;
/// Au-delà, un fichier n'est pas envoyé d'un coup (une photo animée de 200 Mo…).
const ENTIER_MAX: u64 = 64 << 20;
/// Largeur des photos et des pages : l'aperçu fait ~600 px de large, deux fois plus sur écran à haute densité.
const LARGEUR: u32 = 1280;
/// Texte montré au plus (caractères) : autour du passage trouvé.
const TEXTE_MAX: usize = 60_000;
const DOSSIER_MAX: usize = 80;

fn adresse(cle: &str, suite: &str) -> String {
    format!("http://{SCHEMA}.localhost/{cle}{suite}")
}

/// Miroir de `Contenu` dans `src/ipc/apercu.ts`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Contenu {
    Image { adresse: String },
    Video { adresse: String, debut: f64, affiche: String },
    Audio { adresse: String, debut: f64, affiche: String },
    Pdf { adresse: String, page: u32, pages: u32 },
    /// Des blocs de texte (groupes de lignes, diapos, feuilles) ; `cible` : celui à montrer d'abord.
    Texte { blocs: Vec<BlocVu>, cible: usize, lignes: bool },
    Dossier { entrees: Vec<Entree>, total: usize },
    /// Rien de mieux que la grande miniature de Windows.
    Miniature { adresse: String },
}

#[derive(Debug, Serialize)]
pub struct BlocVu {
    /// « diapo 3 », « feuille 2 », ou le numéro de la première ligne.
    titre: String,
    numero: f64,
    texte: String,
}

#[derive(Debug, Serialize)]
pub struct Entree {
    nom: String,
    dossier: bool,
}

/// Miroir de `Apercu` dans `src/ipc/apercu.ts`.
#[derive(Debug, Serialize)]
pub struct Apercu {
    /// « 2,3 Mo · modifié le 3 mars 2024 »
    infos: String,
    contenu: Contenu,
}

/// Les outils d'un seul fil : WIC (photos) et pdfium ne se partagent pas facilement entre fils.
struct Outils {
    images: Option<Images>,
    pdfium: Option<Arc<Pdfium>>,
}

type Travail = Box<dyn FnOnce(&Outils) + Send>;

/// Fait `f` sur le fil des aperçus et attend son résultat.
fn sur_le_fil<R: Send + 'static>(f: impl FnOnce(&Outils) -> R + Send + 'static) -> Option<R> {
    static FIL: OnceLock<Sender<Travail>> = OnceLock::new();
    let fil = FIL.get_or_init(|| {
        let (envoi, reception) = mpsc::channel::<Travail>();
        let lance = std::thread::Builder::new().name("furet-apercus".into()).spawn(move || {
            let (racine, _) = recherche::racine_et_base();
            let outils = Outils { images: Images::nouveau().ok(), pdfium: fichiers::charger_pdfium(&racine).ok() };
            for travail in reception {
                travail(&outils);
            }
        });
        if let Err(e) = lance {
            log::error!("le fil des aperçus n'a pas pu démarrer : {e}");
        }
        envoi
    });
    let (envoi, reception) = mpsc::channel();
    fil.send(Box::new(move |o: &Outils| {
        let _ = envoi.send(f(o));
    }))
    .ok()?;
    reception.recv().ok()
}

fn infos(chemin: &Path) -> String {
    let Ok(m) = std::fs::metadata(chemin) else { return String::new() };
    let modifie = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    // Une photo : quand et où elle a été prise, si on le sait.
    let photo = (Genre::de(chemin) == Some(Genre::Photo)).then(|| recherche::prise_et_lieu(chemin)).flatten();
    let modifie_le = furet_moteur::langue::t("modifié le", "modified");
    let date = photo.or_else(|| modifie.map(|s| format!("{modifie_le} {}", date_lisible(s)))).unwrap_or_default();
    if m.is_dir() {
        return date;
    }
    [taille_lisible(m.len()), date].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// L'aperçu d'un fichier proposé ; `repere` : la page, la ligne ou le moment trouvé.
#[tauri::command]
pub async fn apercu(cle: String, repere: Option<f64>) -> Result<Apercu, String> {
    let chemin = recherche::chemin_offert(&cle).ok_or(furet_moteur::langue::t("fichier inconnu : cherchez à nouveau", "unknown file: search again"))?;
    tauri::async_runtime::spawn_blocking(move || Apercu { infos: infos(&chemin), contenu: contenu(&cle, &chemin, repere) })
        .await
        .map_err(|e| e.to_string())
}

fn contenu(cle: &str, chemin: &Path, repere: Option<f64>) -> Contenu {
    let grande_miniature = || miniatures::adresse_grande(cle);
    if chemin.is_dir() {
        return dossier(chemin);
    }
    match Genre::de(chemin) {
        Some(Genre::Photo) => Contenu::Image { adresse: adresse(cle, "/image") },
        Some(Genre::Video) => Contenu::Video { adresse: adresse(cle, ""), debut: repere.unwrap_or(0.0), affiche: grande_miniature() },
        Some(Genre::Audio) => Contenu::Audio { adresse: adresse(cle, ""), debut: repere.unwrap_or(0.0), affiche: grande_miniature() },
        Some(Genre::Pdf) => {
            let c = chemin.to_path_buf();
            let pages = sur_le_fil(move |o| fichiers::nombre_de_pages(o.pdfium.as_ref()?, &c).ok()).flatten();
            match pages {
                Some(pages) if pages > 0 => Contenu::Pdf {
                    adresse: adresse(cle, "/page/"),
                    page: (repere.unwrap_or(1.0) as u32).clamp(1, pages),
                    pages,
                },
                _ => Contenu::Miniature { adresse: grande_miniature() },
            }
        }
        Some(Genre::Archive) => match fichiers::contenu_archive(chemin, DOSSIER_MAX) {
            Ok((noms, total)) => {
                Contenu::Dossier { entrees: noms.into_iter().map(|nom| Entree { nom, dossier: false }).collect(), total }
            }
            Err(_) => Contenu::Miniature { adresse: grande_miniature() },
        },
        Some(Genre::Document | Genre::Tableur | Genre::Texte | Genre::Code) => match texte(chemin, repere) {
            Some(t) => t,
            None => Contenu::Miniature { adresse: grande_miniature() },
        },
        None => Contenu::Miniature { adresse: grande_miniature() },
    }
}

fn texte(chemin: &Path, repere: Option<f64>) -> Option<Contenu> {
    let blocs = fichiers::texte_pour_apercu(chemin).ok().filter(|b| !b.is_empty())?;
    let lignes = blocs.iter().all(|b| b.nature == "ligne");
    // Le bloc trouvé : celui qui commence au repère, ou le dernier avant.
    let cible = repere.map_or(0, |r| blocs.iter().rposition(|b| b.numero <= r).unwrap_or(0));
    // Quelques blocs avant, puis la suite tant que ça reste raisonnable.
    let debut = cible.saturating_sub(3);
    let mut taille = 0;
    let mut vus = Vec::new();
    for b in blocs.into_iter().skip(debut) {
        taille += b.texte.len();
        if taille > TEXTE_MAX && !vus.is_empty() {
            break;
        }
        let titre = match b.nature {
            "diapo" => format!("{} {}", furet_moteur::langue::t("diapo", "slide"), b.numero as u32),
            "feuille" => format!("{} {}", furet_moteur::langue::t("feuille", "sheet"), b.numero as u32),
            _ => String::new(),
        };
        vus.push(BlocVu { titre, numero: b.numero, texte: b.texte });
    }
    Some(Contenu::Texte { blocs: vus, cible: cible - debut, lignes })
}

fn dossier(chemin: &Path) -> Contenu {
    let mut entrees: Vec<Entree> = std::fs::read_dir(chemin)
        .map(|l| {
            l.flatten()
                .map(|e| Entree {
                    nom: e.file_name().to_string_lossy().into_owned(),
                    dossier: e.file_type().is_ok_and(|t| t.is_dir()),
                })
                .collect()
        })
        .unwrap_or_default();
    let total = entrees.len();
    entrees.sort_by_cached_key(|e| (!e.dossier, e.nom.to_lowercase()));
    entrees.truncate(DOSSIER_MAX);
    Contenu::Dossier { entrees, total }
}

/// Répond à une demande `furet-fichier`.
pub fn servir(demande: Request<Vec<u8>>, repondeur: tauri::UriSchemeResponder) {
    std::thread::spawn(move || {
        let chemin = demande.uri().path().trim_start_matches('/').to_owned();
        let mut parties = chemin.split('/');
        let cle = parties.next().unwrap_or_default();
        let reponse = match (recherche::chemin_offert(cle), parties.next(), parties.next()) {
            (None, _, _) => vide(StatusCode::NOT_FOUND),
            (Some(f), None, _) => fichier(&f, &demande),
            (Some(f), Some("image"), _) => jpeg(sur_le_fil(move |o| {
                let images = o.images.as_ref()?;
                images.ouvrir(&f).and_then(|s| images.jpeg(&s, LARGEUR)).ok()
            })),
            (Some(f), Some("page"), Some(n)) => {
                let n: u32 = n.parse().unwrap_or(1);
                jpeg(sur_le_fil(move |o| fichiers::rendre_page(o.pdfium.as_ref()?, &f, n, LARGEUR).ok().map(|(j, _)| j)))
            }
            _ => vide(StatusCode::NOT_FOUND),
        };
        repondeur.respond(reponse);
    });
}

fn vide(statut: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(statut).body(Vec::new()).unwrap_or_else(|_| Response::new(Vec::new()))
}

fn jpeg(octets: Option<Option<Vec<u8>>>) -> Response<Vec<u8>> {
    match octets.flatten() {
        Some(j) => Response::builder()
            .header(header::CONTENT_TYPE, "image/jpeg")
            .header(header::CACHE_CONTROL, "max-age=600")
            .body(j)
            .unwrap_or_else(|_| Response::new(Vec::new())),
        None => vide(StatusCode::NOT_FOUND),
    }
}

/// Le type annoncé au lecteur. Un .mov ou un .mkv en H.264 se lit souvent comme un .mp4 ou un .webm.
fn type_mime(chemin: &Path) -> &'static str {
    let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" | "mov" => "video/mp4",
        "webm" | "mkv" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" | "aac" => "audio/mp4",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// « bytes=100-199 » ou « bytes=100- » → (100, Some(199)) ou (100, None).
fn plage(demande: &Request<Vec<u8>>) -> Option<(u64, Option<u64>)> {
    let valeur = demande.headers().get(header::RANGE)?.to_str().ok()?.strip_prefix("bytes=")?;
    let (a, b) = valeur.split(',').next()?.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()))
}

/// Le fichier, en entier ou la plage demandée (le lecteur vidéo avance par plages).
fn fichier(chemin: &PathBuf, demande: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let lu = (|| -> std::io::Result<Response<Vec<u8>>> {
        let mut f = File::open(chemin)?;
        let total = f.metadata()?.len();
        let base = Response::builder().header(header::CONTENT_TYPE, type_mime(chemin)).header(header::ACCEPT_RANGES, "bytes");
        let (debut, fin, partiel) = match plage(demande) {
            Some((a, b)) if a < total => (a, b.unwrap_or(u64::MAX).min(total - 1).min(a + MORCEAU - 1), true),
            Some(_) => {
                return Ok(base
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(Vec::new())
                    .unwrap_or_default());
            }
            None if total <= ENTIER_MAX => (0, total.saturating_sub(1), false),
            None => (0, MORCEAU - 1, true),
        };
        let mut octets = vec![0; (fin + 1 - debut) as usize];
        if total > 0 {
            f.seek(SeekFrom::Start(debut))?;
            f.read_exact(&mut octets)?;
        }
        let r = if partiel {
            base.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {debut}-{fin}/{total}"))
        } else {
            base
        };
        Ok(r.header(header::CONTENT_LENGTH, octets.len()).body(octets).unwrap_or_default())
    })();
    lu.unwrap_or_else(|_| vide(StatusCode::NOT_FOUND))
}
