//! Vidéo : une image par seconde avec ffmpeg, découpée en plans.

use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

/// Différence moyenne (0 à 1) entre deux images au-delà de laquelle on change de plan.
const SEUIL_PLAN: f32 = 0.12;
/// En dessous, deux images sont identiques (ffmpeg répète la dernière image clé pour combler les trous).
const SEUIL_IDENTIQUE: f32 = 0.004;
/// Un plan plus long est coupé, pour pouvoir ouvrir la vidéo au bon moment.
const PLAN_MAX_SECONDES: usize = 30;
/// Images envoyées au modèle pour un plan.
const IMAGES_PAR_PLAN: usize = 8;
/// On ne décode que les images clés si elles sont espacées de 2 s au plus (vidéos de téléphone, de caméra…).
const ECART_IMAGES_CLES_MAX: f64 = 2.0;
/// Une vidéo courte se décode vite : on la décode entièrement.
const DUREE_MIN_IMAGES_CLES: f64 = 20.0;

pub struct Plan {
    pub debut: f64,
    pub fin: f64,
    pub images: Vec<Vec<u8>>,
}

/// Plans de la vidéo. Quand les images clés sont rapprochées, on ne décode qu'elles (6 fois plus rapide).
pub fn plans(chemin: &Path) -> Result<Vec<Plan>> {
    if let Some((duree, images_cles)) = images_cles(chemin) {
        if duree >= DUREE_MIN_IMAGES_CLES && images_cles > 0 && duree / images_cles as f64 <= ECART_IMAGES_CLES_MAX {
            if let Ok(plans) = decouper(chemin, true) {
                if !plans.is_empty() {
                    return Ok(plans);
                }
            }
        }
    }
    decouper(chemin, false)
}

/// Durée (s) et nombre d'images clés, lus dans le conteneur sans rien décoder.
fn images_cles(chemin: &Path) -> Option<(f64, usize)> {
    let sortie = crate::processus::commande_basse(crate::processus::ffmpeg("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=flags:format=duration", "-of", "csv=p=0"])
        .arg(chemin)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let texte = String::from_utf8_lossy(&sortie.stdout);
    let images_cles = texte.lines().filter(|l| l.starts_with('K')).count();
    let duree = texte.lines().filter_map(|l| l.trim().parse::<f64>().ok()).last()?;
    Some((duree, images_cles))
}

fn decouper(chemin: &Path, images_cles: bool) -> Result<Vec<Plan>> {
    let mut ffmpeg = crate::processus::commande_basse(crate::processus::ffmpeg("ffmpeg"));
    ffmpeg.args(["-v", "error", "-nostdin", "-hwaccel", "auto"]);
    if images_cles {
        ffmpeg.args(["-skip_frame", "nokey"]);
    }
    let mut ffmpeg = ffmpeg
        .arg("-i")
        .arg(chemin)
        .args(["-an", "-sn", "-dn", "-vf", "fps=1,scale=768:768:force_original_aspect_ratio=decrease"])
        .args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("lancement de ffmpeg")?;
    let mut sortie = ffmpeg.stdout.take().context("sortie de ffmpeg")?;

    let mut plans = Vec::new();
    let mut courant: Vec<Vec<u8>> = Vec::new();
    let mut debut_courant = 0usize;
    let mut seconde = 0usize;
    let mut precedente: Option<Vec<u8>> = None;
    let mut tampon = Vec::new();
    let mut morceau = vec![0u8; 1 << 16];
    loop {
        let n = sortie.read(&mut morceau)?;
        if n == 0 {
            break;
        }
        tampon.extend_from_slice(&morceau[..n]);
        // Chaque image JPEG se termine par FF D9.
        while let Some(fin) = tampon.windows(2).position(|w| w == [0xFF, 0xD9]) {
            let jpeg: Vec<u8> = tampon.drain(..fin + 2).collect();
            let signature = signature(&jpeg)?;
            let ecart = precedente.as_ref().map(|p| difference(p, &signature));
            if !courant.is_empty() && (ecart.is_some_and(|e| e > SEUIL_PLAN) || seconde - debut_courant >= PLAN_MAX_SECONDES) {
                plans.push(fermer(debut_courant, seconde, std::mem::take(&mut courant)));
                debut_courant = seconde;
            }
            // Une image identique à la précédente n'apporte rien au modèle.
            if courant.is_empty() || ecart.is_none_or(|e| e > SEUIL_IDENTIQUE) {
                courant.push(jpeg);
            }
            precedente = Some(signature);
            seconde += 1;
        }
    }
    if !ffmpeg.wait()?.success() && plans.is_empty() && courant.is_empty() {
        bail!("ffmpeg n'a pas pu lire la vidéo");
    }
    if !courant.is_empty() {
        plans.push(fermer(debut_courant, seconde, courant));
    }
    Ok(plans)
}

fn fermer(debut: usize, fin: usize, images: Vec<Vec<u8>>) -> Plan {
    let n = images.len();
    let images = if n <= IMAGES_PAR_PLAN {
        images
    } else {
        let gardees: Vec<usize> = (0..IMAGES_PAR_PLAN).map(|i| i * n / IMAGES_PAR_PLAN).collect();
        images.into_iter().enumerate().filter(|(i, _)| gardees.contains(i)).map(|(_, j)| j).collect()
    };
    Plan { debut: debut as f64, fin: fin as f64, images }
}

/// Miniature 32×18 en niveaux de gris, pour comparer deux images rapidement.
fn signature(jpeg: &[u8]) -> Result<Vec<u8>> {
    let image = ::image::load_from_memory_with_format(jpeg, ::image::ImageFormat::Jpeg)?;
    Ok(image.resize_exact(32, 18, ::image::imageops::FilterType::Triangle).to_luma8().into_raw())
}

fn difference(a: &[u8], b: &[u8]) -> f32 {
    let somme: u32 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u32).sum();
    somme as f32 / (a.len().max(1) as f32 * 255.0)
}
