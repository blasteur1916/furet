//! Ce que Furet télécharge au premier lancement : le moteur (llama.cpp), les modèles, pdfium, ffmpeg et la
//! liste des villes. Chaque fichier est vérifié (SHA-256) ; un téléchargement coupé reprend où il en était.
//! On ne garde des archives que ce qui sert. La liste des paquets est propre à Windows (Linux : étape à part).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Pourquoi un paquet est téléchargé.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Groupe {
    /// Sans eux, rien ne marche.
    Essentiel,
    /// La parole des sons et des vidéos (réglage « Transcrire »).
    Transcription,
    /// La reconnaissance des visages (réglage « Visages »).
    Visages,
}

/// Comment ranger ce qui est téléchargé.
#[derive(Clone, Copy, Debug)]
enum Forme {
    /// Un fichier tel quel, à ce chemin (relatif à la racine).
    Fichier(&'static str),
    /// Une archive : `garder` dit où ranger chaque fichier qui sert (chemin relatif à la racine), ou rien.
    Zip(fn(&str) -> Option<String>),
    TarGz(fn(&str) -> Option<String>),
    TarBz2(fn(&str) -> Option<String>),
    /// Les villes et les pays (plusieurs fichiers de GeoNames et du CLDR, mis en forme ici).
    Lieux,
}

#[derive(Clone, Copy, Debug)]
pub struct Paquet {
    /// Ce que voit l'utilisateur, en français et en anglais (voir `nom()`).
    nom_fr: &'static str,
    nom_en: &'static str,
    pub groupe: Groupe,
    url: &'static str,
    /// Octets à télécharger.
    pub taille: u64,
    /// Rien pour les fichiers de GeoNames, refaits chaque jour.
    sha256: Option<&'static str>,
    forme: Forme,
    /// Ce fichier (relatif à la racine) prouve que le paquet est installé.
    present: &'static str,
}

const LLAMA_GARDER: fn(&str) -> bool = |nom| {
    let nom = nom.rsplit('/').next().unwrap_or(nom);
    nom == "llama-server.exe"
        || nom.starts_with("LICENSE")
        || (nom.ends_with(".dll") && (!nom.contains("-impl") || nom == "llama-server-impl.dll"))
};

fn dernier(chemin: &str) -> &str {
    chemin.rsplit('/').next().unwrap_or(chemin)
}

#[cfg(windows)]
pub const PAQUETS: [Paquet; 11] = [
    Paquet {
        nom_fr: "Moteur de recherche (processeur)",
        nom_en: "Search engine (CPU)",
        groupe: Groupe::Essentiel,
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b12202/llama-b12202-bin-win-cpu-x64.zip",
        taille: 24_759_273,
        sha256: Some("75d108e6f91907638fef0cbdf4cc230e56bd9eac71bbffb209ae8da1f932590e"),
        forme: Forme::Zip(|n| LLAMA_GARDER(n).then(|| format!("outils/llama-cpu/{}", dernier(n)))),
        present: "outils/llama-cpu/llama-server.exe",
    },
    Paquet {
        nom_fr: "Moteur de recherche (carte graphique)",
        nom_en: "Search engine (GPU)",
        groupe: Groupe::Essentiel,
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b12202/llama-b12202-bin-win-vulkan-x64.zip",
        taille: 37_715_166,
        sha256: Some("ab526ddd4cacca463b73a385ede310c9888818d084efb57f3d113b4d3e79c404"),
        forme: Forme::Zip(|n| LLAMA_GARDER(n).then(|| format!("outils/llama-vulkan/{}", dernier(n)))),
        present: "outils/llama-vulkan/llama-server.exe",
    },
    Paquet {
        nom_fr: "Modèle EmbeddingGemma 2",
        nom_en: "EmbeddingGemma 2 model",
        groupe: Groupe::Essentiel,
        url: "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/embeddinggemma-2-Q8_0.gguf",
        taille: 309_855_456,
        sha256: Some("2188ac1deca4b77dffefd603c2776a9d76d9d74ec01841392982ebb840b09135"),
        forme: Forme::Fichier("modeles/embeddinggemma-2-Q8_0.gguf"),
        present: "modeles/embeddinggemma-2-Q8_0.gguf",
    },
    Paquet {
        nom_fr: "Modèle EmbeddingGemma 2 (images et sons)",
        nom_en: "EmbeddingGemma 2 model (images and sound)",
        groupe: Groupe::Essentiel,
        url: "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/mmproj-embeddinggemma-2-Q8_0.gguf",
        taille: 554_821_056,
        sha256: Some("0531309472a6bc6f4b146c3736eee7795ec9db1903c25490d3c937490cef6dd8"),
        forme: Forme::Fichier("modeles/mmproj-embeddinggemma-2-Q8_0.gguf"),
        present: "modeles/mmproj-embeddinggemma-2-Q8_0.gguf",
    },
    Paquet {
        nom_fr: "Lecteur de PDF (PDFium)",
        nom_en: "PDF reader (PDFium)",
        groupe: Groupe::Essentiel,
        url: "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8138/pdfium-win-x64.tgz",
        taille: 4_031_929,
        sha256: Some("8d48973e3eb472dda0f5f2435e595ac043f9f2219dd71883392f549445afa8a5"),
        forme: Forme::TarGz(|n| match n.trim_start_matches("./") {
            "bin/pdfium.dll" => Some("outils/pdfium/bin/pdfium.dll".into()),
            "LICENSE" => Some("outils/pdfium/LICENSE".into()),
            _ => None,
        }),
        present: "outils/pdfium/bin/pdfium.dll",
    },
    Paquet {
        nom_fr: "Lecteur de sons et de vidéos (FFmpeg)",
        nom_en: "Audio and video reader (FFmpeg)",
        groupe: Groupe::Essentiel,
        url: "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
        taille: 114_768_076,
        sha256: Some("60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba"),
        forme: Forme::Zip(|n| match dernier(n) {
            f @ ("ffmpeg.exe" | "ffprobe.exe") if n.contains("/bin/") => Some(format!("outils/ffmpeg/{f}")),
            "LICENSE" => Some("outils/ffmpeg/LICENSE".into()),
            _ => None,
        }),
        present: "outils/ffmpeg/ffprobe.exe",
    },
    Paquet {
        nom_fr: "Villes du monde (GeoNames)",
        nom_en: "World cities (GeoNames)",
        groupe: Groupe::Essentiel,
        url: "https://download.geonames.org/export/dump/cities15000.zip",
        taille: 3_600_000,
        sha256: None,
        forme: Forme::Lieux,
        present: "modeles/lieux/villes.tsv",
    },
    Paquet {
        nom_fr: "Transcription (sherpa-onnx)",
        nom_en: "Speech recognition (sherpa-onnx)",
        groupe: Groupe::Transcription,
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts.tar.bz2",
        taille: 20_962_760,
        sha256: Some("11fd6a50b97eed38b54fb16d8219f49b16aa7f6a8e43a042316469ef47652e82"),
        forme: Forme::TarBz2(|n| {
            let f = dernier(n);
            let sert = (n.contains("/bin/") && (f.ends_with(".dll") || f == "sherpa-onnx-offline-websocket-server.exe"))
                || f.starts_with("LICENSE");
            sert.then(|| format!("outils/sherpa-onnx/{}", n.trim_start_matches("./")))
        }),
        present: "outils/sherpa-onnx/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-offline-websocket-server.exe",
    },
    Paquet {
        nom_fr: "Modèle de transcription Parakeet v3",
        nom_en: "Parakeet v3 speech model",
        groupe: Groupe::Transcription,
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
        taille: 487_211_443,
        sha256: Some("1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9"),
        forme: Forme::TarBz2(|n| {
            let f = dernier(n);
            (f.ends_with(".onnx") || f == "tokens.txt" || f == "README.md")
                .then(|| format!("modeles/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/{f}"))
        }),
        present: "modeles/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/encoder.int8.onnx",
    },
    Paquet {
        nom_fr: "Détection des visages (YuNet)",
        nom_en: "Face detection (YuNet)",
        groupe: Groupe::Visages,
        url: "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx",
        taille: 232_589,
        sha256: Some("8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"),
        forme: Forme::Fichier("modeles/visages/face_detection_yunet_2023mar.onnx"),
        present: "modeles/visages/face_detection_yunet_2023mar.onnx",
    },
    Paquet {
        nom_fr: "Reconnaissance des visages (SFace)",
        nom_en: "Face recognition (SFace)",
        groupe: Groupe::Visages,
        url: "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx",
        taille: 38_696_353,
        sha256: Some("0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79"),
        forme: Forme::Fichier("modeles/visages/face_recognition_sface_2021dec.onnx"),
        present: "modeles/visages/face_recognition_sface_2021dec.onnx",
    },
];

#[cfg(not(windows))]
pub const PAQUETS: [Paquet; 0] = [];

/// Les fichiers de GeoNames et du CLDR dont on tire `modeles/lieux`.
const LIEUX_SOURCES: [(&str, &str); 4] = [
    ("cities15000.zip", "https://download.geonames.org/export/dump/cities15000.zip"),
    ("admin1CodesASCII.txt", "https://download.geonames.org/export/dump/admin1CodesASCII.txt"),
    ("countryInfo.txt", "https://download.geonames.org/export/dump/countryInfo.txt"),
    (
        "territories_fr.json",
        "https://raw.githubusercontent.com/unicode-org/cldr-json/main/cldr-json/cldr-localenames-full/main/fr/territories.json",
    ),
];

impl Paquet {
    pub fn nom(&self) -> &'static str {
        crate::langue::t(self.nom_fr, self.nom_en)
    }

    pub fn est_installe(&self, racine: &Path) -> bool {
        racine.join(self.present).exists()
    }
}

/// Les paquets de ces groupes qui ne sont pas encore là.
pub fn manquants(racine: &Path, groupes: &[Groupe]) -> Vec<Paquet> {
    PAQUETS.iter().filter(|p| groupes.contains(&p.groupe) && !p.est_installe(racine)).copied().collect()
}

/// L'avancement : octets reçus en tout, sur le total, et le paquet en cours.
#[derive(Clone, Debug)]
pub struct Avancement {
    pub fait: u64,
    pub total: u64,
    pub nom: &'static str,
}

/// Télécharge et installe ces paquets, l'un après l'autre. `arret` les interrompt : la suite reprendra.
pub fn installer(
    racine: &Path,
    paquets: &[Paquet],
    arret: &AtomicBool,
    suivi: &mut dyn FnMut(Avancement),
) -> Result<()> {
    let total: u64 = paquets.iter().map(|p| p.taille).sum();
    let mut avant = 0;
    let dossier = racine.join("telechargements");
    fs::create_dir_all(&dossier)?;
    for p in paquets {
        let mut sur = |fait: u64| suivi(Avancement { fait: avant + fait.min(p.taille), total, nom: p.nom() });
        sur(0);
        match p.forme {
            Forme::Lieux => installer_lieux(racine, &dossier, arret)?,
            forme => {
                let nom = p.url.rsplit('/').next().unwrap_or("paquet");
                let archive = dossier.join(nom);
                telecharger(p.url, &archive, arret, &mut sur)?;
                if let Some(attendu) = p.sha256 {
                    let lu = empreinte(&archive)?;
                    if lu != attendu {
                        let _ = fs::remove_file(&archive);
                        bail!(
                            "{}",
                            crate::langue::t("« {} » est arrivé abîmé : il sera retéléchargé", "“{}” arrived damaged: it will be downloaded again")
                                .replace("{}", p.nom())
                        );
                    }
                }
                ranger(racine, &archive, forme).with_context(|| format!("{} {}", crate::langue::t("installation de", "installing"), p.nom()))?;
                let _ = fs::remove_file(&archive);
            }
        }
        avant += p.taille;
        suivi(Avancement { fait: avant, total, nom: p.nom() });
    }
    let _ = fs::remove_dir(&dossier);
    Ok(())
}

/// Télécharge `url` dans `fichier`, en reprenant un téléchargement coupé (le morceau déjà reçu est gardé
/// dans `<fichier>.partiel`).
fn telecharger(url: &str, fichier: &Path, arret: &AtomicBool, suivi: &mut dyn FnMut(u64)) -> Result<()> {
    if fichier.exists() {
        return Ok(());
    }
    let partiel = fichier.with_extension(format!(
        "{}.partiel",
        fichier.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    let deja = fs::metadata(&partiel).map_or(0, |m| m.len());
    let mut requete = ureq::get(url).header("User-Agent", "Furet (recherche de fichiers)");
    if deja > 0 {
        requete = requete.header("Range", format!("bytes={deja}-"));
    }
    let reponse = requete.call().with_context(|| format!("téléchargement de {url}"))?;
    // 206 : la suite ; 200 : le serveur renvoie tout, on repart de zéro.
    let (mut sortie, mut fait) = if reponse.status().as_u16() == 206 {
        (OpenOptions::new().append(true).open(&partiel)?, deja)
    } else {
        (File::create(&partiel)?, 0)
    };
    let mut lecteur = reponse.into_body().into_reader();
    let mut tampon = vec![0u8; 1 << 18];
    loop {
        if arret.load(Ordering::Relaxed) {
            bail!("téléchargement interrompu");
        }
        let n = lecteur.read(&mut tampon)?;
        if n == 0 {
            break;
        }
        sortie.write_all(&tampon[..n])?;
        fait += n as u64;
        suivi(fait);
    }
    sortie.flush()?;
    drop(sortie);
    fs::rename(&partiel, fichier)?;
    Ok(())
}

fn empreinte(fichier: &Path) -> Result<String> {
    let mut f = File::open(fichier)?;
    let mut h = Sha256::new();
    let mut tampon = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut tampon)?;
        if n == 0 {
            break;
        }
        h.update(&tampon[..n]);
    }
    Ok(h.finalize().iter().map(|o| format!("{o:02x}")).collect())
}

/// Écrit `lecteur` dans `racine/relatif` (dans un fichier provisoire d'abord : un arrêt en plein milieu ne
/// laisse pas un fichier tronqué qui aurait l'air installé).
fn ecrire(racine: &Path, relatif: &str, lecteur: &mut dyn Read) -> Result<()> {
    // Les chemins viennent de nos propres règles `garder`, jamais tels quels de l'archive.
    if relatif.split('/').any(|p| p == ".." || p.is_empty()) {
        bail!("chemin refusé : {relatif}");
    }
    let cible: PathBuf = racine.join(relatif);
    if let Some(d) = cible.parent() {
        fs::create_dir_all(d)?;
    }
    let provisoire = cible.with_extension("installation");
    std::io::copy(lecteur, &mut File::create(&provisoire)?)?;
    fs::rename(&provisoire, &cible)?;
    Ok(())
}

fn ranger(racine: &Path, archive: &Path, forme: Forme) -> Result<()> {
    let mut ranges = 0;
    match forme {
        Forme::Fichier(relatif) => {
            ecrire(racine, relatif, &mut File::open(archive)?)?;
            ranges += 1;
        }
        Forme::Zip(garder) => {
            let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
            for i in 0..zip.len() {
                let mut entree = zip.by_index(i)?;
                if entree.is_dir() {
                    continue;
                }
                let nom = entree.name()?.replace('\\', "/");
                if let Some(relatif) = garder(&nom) {
                    ecrire(racine, &relatif, &mut entree)?;
                    ranges += 1;
                }
            }
        }
        Forme::TarGz(garder) | Forme::TarBz2(garder) => {
            let f = File::open(archive)?;
            let flux: Box<dyn Read> = match forme {
                Forme::TarGz(_) => Box::new(flate2::read::GzDecoder::new(f)),
                _ => Box::new(bzip2::read::BzDecoder::new(f)),
            };
            let mut tar = tar::Archive::new(flux);
            for entree in tar.entries()? {
                let mut entree = entree?;
                if !entree.header().entry_type().is_file() {
                    continue;
                }
                let nom = entree.path()?.to_string_lossy().replace('\\', "/");
                if let Some(relatif) = garder(&nom) {
                    ecrire(racine, &relatif, &mut entree)?;
                    ranges += 1;
                }
            }
        }
        Forme::Lieux => unreachable!("rangé à part"),
    }
    if ranges == 0 {
        bail!("rien d'utile dans l'archive");
    }
    Ok(())
}

fn installer_lieux(racine: &Path, dossier: &Path, arret: &AtomicBool) -> Result<()> {
    let source = dossier.join("lieux");
    fs::create_dir_all(&source)?;
    for (nom, url) in LIEUX_SOURCES {
        telecharger(url, &source.join(nom), arret, &mut |_| {})?;
    }
    crate::lieux::preparer(&source, &racine.join("modeles").join("lieux"))?;
    let _ = fs::remove_dir_all(&source);
    Ok(())
}
