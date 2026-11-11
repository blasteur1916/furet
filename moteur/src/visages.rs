//! Reconnaître les personnes sur les photos, sur ce PC uniquement.
//!
//! - YuNet (OpenCV, MIT) trouve les visages et 5 repères (yeux, nez, coins de la bouche).
//! - Chaque visage est redressé d'après ces repères, puis SFace (OpenCV, Apache 2.0) en fait une empreinte
//!   de 128 nombres : proche d'une photo à l'autre pour la même personne, éloignée pour une autre.
//! - Une photo qui ne montre qu'un visage et dont le nom ressemble à un prénom (« elliot.png ») donne son
//!   nom à ce visage. Chercher « elliot » trouve alors toutes les photos où ce visage revient, même
//!   « 234grehrh45.png ».
//!
//! Les deux modèles tournent avec tract (100 % Rust, aussi sous Linux), sur le processeur.

use crate::noms::normaliser;
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tract_onnx::prelude::*;

pub const DIMENSION: usize = 128;
/// Côté de l'image donnée au détecteur (son entrée est fixe).
pub const COTE: usize = 640;
/// Confiance minimale d'un visage (YuNet).
const SEUIL_DETECTION: f32 = 0.85;
/// En dessous (en pixels, dans l'image de 640), le visage est trop petit pour être reconnu.
const COTE_MIN: f32 = 28.0;
/// Deux empreintes plus proches que ça (cosinus) : la même personne. OpenCV conseille 0,363 pour SFace ;
/// un peu plus strict, pour ne pas mélanger deux personnes.
pub const SEUIL_MEME_PERSONNE: f32 = 0.40;
/// Où SFace attend les 5 repères dans son image de 112 × 112.
const MODELE_ALIGNEMENT: [[f32; 2]; 5] =
    [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];
/// Des noms de photos qui ne sont pas des prénoms.
const PAS_DES_PRENOMS: [&str; 40] = [
    "img", "image", "photo", "photos", "pic", "picture", "screenshot", "capture", "ecran", "scan", "dsc", "dcim",
    "wallpaper", "fond", "sans", "titre", "untitled", "copie", "copy", "export", "edit", "final", "test", "avatar",
    "profil", "profile", "selfie", "portrait", "groupe", "famille", "family", "vacances", "mariage", "anniversaire",
    "noel", "logo", "icon", "icone", "banner", "cover",
];

type Plan = Arc<TypedRunnableModel>;

/// Un visage trouvé : sa place dans l'image réduite (x, y, largeur, hauteur) et son empreinte (normée).
pub struct Visage {
    pub boite: [f32; 4],
    pub empreinte: Vec<f32>,
}

/// Les deux modèles, chargés une fois et partagés entre les fils.
pub struct Reconnaissance {
    detecteur: Plan,
    empreinte: Plan,
}

impl Reconnaissance {
    pub fn charger(racine: &Path) -> Result<Arc<Self>> {
        static CHARGEE: OnceLock<Arc<Reconnaissance>> = OnceLock::new();
        if let Some(r) = CHARGEE.get() {
            return Ok(r.clone());
        }
        let dossier = racine.join("modeles").join("visages");
        let plan = |nom: &str, forme: [usize; 4]| -> Result<Plan> {
            let p = tract_onnx::onnx()
                .model_for_path(dossier.join(nom))
                .and_then(|m| m.with_input_fact(0, f32::fact(forme).into()))
                .and_then(|m| m.into_optimized())
                .and_then(|m| m.into_runnable())
                .map_err(|e| anyhow!("{nom} : {e}"))?;
            Ok(p)
        };
        let r = Arc::new(Reconnaissance {
            detecteur: plan("face_detection_yunet_2023mar.onnx", [1, 3, COTE, COTE])?,
            empreinte: plan("face_recognition_sface_2021dec.onnx", [1, 3, 112, 112])?,
        });
        Ok(CHARGEE.get_or_init(|| r).clone())
    }

    /// Les visages d'une image BGR (3 octets par point) qui tient dans COTE × COTE.
    pub fn analyser(&self, bgr: &[u8], largeur: usize, hauteur: usize) -> Result<Vec<Visage>> {
        if largeur > COTE || hauteur > COTE || bgr.len() < largeur * hauteur * 3 {
            return Err(anyhow!("image trop grande pour le détecteur"));
        }
        // L'image en haut à gauche d'un carré de 640, le reste à zéro ; BGR 0–255 comme l'attend YuNet.
        let mut entree = vec![0f32; 3 * COTE * COTE];
        for y in 0..hauteur {
            for x in 0..largeur {
                let p = (y * largeur + x) * 3;
                for c in 0..3 {
                    entree[c * COTE * COTE + y * COTE + x] = bgr[p + c] as f32;
                }
            }
        }
        let tenseur = Tensor::from_shape(&[1, 3, COTE, COTE], &entree).map_err(|e| anyhow!("{e}"))?;
        let sorties = self.detecteur.run(tvec!(tenseur.into())).map_err(|e| anyhow!("YuNet : {e}"))?;
        let lire = |i: usize| -> Result<Vec<f32>> {
            Ok(sorties[i].to_plain_array_view::<f32>().map_err(|e| anyhow!("{e}"))?.iter().copied().collect())
        };

        // Décodage de YuNet : 3 échelles (pas de 8, 16, 32) ; pour chacune, confiance, objet, boîte, repères.
        let mut trouves: Vec<(f32, [f32; 4], [[f32; 2]; 5])> = Vec::new();
        for (k, pas) in [8usize, 16, 32].into_iter().enumerate() {
            let (cls, obj, boites, reperes) = (lire(k)?, lire(3 + k)?, lire(6 + k)?, lire(9 + k)?);
            let colonnes = COTE / pas;
            let s = pas as f32;
            for i in 0..cls.len() {
                let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
                if score < SEUIL_DETECTION {
                    continue;
                }
                let (r, c) = ((i / colonnes) as f32, (i % colonnes) as f32);
                let (cx, cy) = ((c + boites[i * 4]) * s, (r + boites[i * 4 + 1]) * s);
                let (l, h) = (boites[i * 4 + 2].exp() * s, boites[i * 4 + 3].exp() * s);
                let mut pts = [[0f32; 2]; 5];
                for (n, p) in pts.iter_mut().enumerate() {
                    *p = [(reperes[i * 10 + 2 * n] + c) * s, (reperes[i * 10 + 2 * n + 1] + r) * s];
                }
                trouves.push((score, [cx - l / 2.0, cy - h / 2.0, l, h], pts));
            }
        }
        // Un même visage est vu plusieurs fois : on garde le plus sûr (suppression des chevauchements).
        trouves.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut gardes: Vec<(f32, [f32; 4], [[f32; 2]; 5])> = Vec::new();
        for t in trouves {
            if t.1[2] >= COTE_MIN && t.1[3] >= COTE_MIN && gardes.iter().all(|g| recouvrement(&g.1, &t.1) < 0.3) {
                gardes.push(t);
            }
        }
        let mut visages = Vec::with_capacity(gardes.len());
        for (_, boite, pts) in gardes {
            let aligne = aligner(bgr, largeur, hauteur, &pts);
            let tenseur = Tensor::from_shape(&[1, 3, 112, 112], &aligne).map_err(|e| anyhow!("{e}"))?;
            let sortie = self.empreinte.run(tvec!(tenseur.into())).map_err(|e| anyhow!("SFace : {e}"))?;
            let mut e: Vec<f32> =
                sortie[0].to_plain_array_view::<f32>().map_err(|e| anyhow!("{e}"))?.iter().copied().collect();
            let norme = e.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
            e.iter_mut().for_each(|x| *x /= norme);
            visages.push(Visage { boite, empreinte: e });
        }
        Ok(visages)
    }
}

/// Part commune de deux boîtes (intersection sur union).
fn recouvrement(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let l = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if l <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let inter = l * h;
    inter / (a[2] * a[3] + b[2] * b[3] - inter)
}

/// Le visage redressé en 112 × 112, RVB, 0–255, comme l'attend SFace : la similitude (rotation, échelle,
/// déplacement) qui amène au mieux les 5 repères sur leur place type, appliquée à l'envers point par point.
fn aligner(bgr: &[u8], largeur: usize, hauteur: usize, pts: &[[f32; 2]; 5]) -> Vec<f32> {
    let n = 5.0;
    let (msx, msy) = (pts.iter().map(|p| p[0]).sum::<f32>() / n, pts.iter().map(|p| p[1]).sum::<f32>() / n);
    let (mdx, mdy) =
        (MODELE_ALIGNEMENT.iter().map(|p| p[0]).sum::<f32>() / n, MODELE_ALIGNEMENT.iter().map(|p| p[1]).sum::<f32>() / n);
    // Moindres carrés pour [[a, -b], [b, a]] : la meilleure similitude sans miroir.
    let (mut num_a, mut num_b, mut den) = (0f32, 0f32, 0f32);
    for (s, d) in pts.iter().zip(MODELE_ALIGNEMENT) {
        let (sx, sy, dx, dy) = (s[0] - msx, s[1] - msy, d[0] - mdx, d[1] - mdy);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        den += sx * sx + sy * sy;
    }
    let (a, b) = (num_a / den.max(1e-6), num_b / den.max(1e-6));
    let (tx, ty) = (mdx - (a * msx - b * msy), mdy - (b * msx + a * msy));
    let det = (a * a + b * b).max(1e-12);
    let mut sortie = vec![0f32; 3 * 112 * 112];
    for v in 0..112 {
        for u in 0..112 {
            // Point de l'image d'origine qui arrive en (u, v).
            let (px, py) = (u as f32 - tx, v as f32 - ty);
            let (x, y) = ((a * px + b * py) / det, (-b * px + a * py) / det);
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            for c in 0..3 {
                let pixel = |xx: f32, yy: f32| -> f32 {
                    if xx < 0.0 || yy < 0.0 || xx >= largeur as f32 || yy >= hauteur as f32 {
                        0.0
                    } else {
                        // BGR → RVB.
                        bgr[(yy as usize * largeur + xx as usize) * 3 + (2 - c)] as f32
                    }
                };
                let valeur = pixel(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + pixel(x0 + 1.0, y0) * fx * (1.0 - fy)
                    + pixel(x0, y0 + 1.0) * (1.0 - fx) * fy
                    + pixel(x0 + 1.0, y0 + 1.0) * fx * fy;
                sortie[c * 112 * 112 + v * 112 + u] = valeur;
            }
        }
    }
    sortie
}

/// « elliot.png » → Some("elliot") ; « IMG_2045.jpg », « capture écran », « 234grehrh45 » → None.
pub fn prenom_du_fichier(chemin: &Path) -> Option<String> {
    let nom = chemin.file_stem()?.to_string_lossy().to_string();
    let cle = normaliser(&nom);
    let mots: Vec<&str> = cle.split(' ').filter(|m| !m.is_empty()).collect();
    let lettres: usize = mots.iter().map(|m| m.chars().count()).sum();
    let ressemble = (1..=3).contains(&mots.len())
        && (3..=30).contains(&lettres)
        && mots.iter().all(|m| m.chars().all(char::is_alphabetic) && !PAS_DES_PRENOMS.contains(m));
    ressemble.then_some(cle)
}

fn octets(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn depuis_octets(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Enregistre les visages d'un fichier ; s'il n'y en a qu'un et que le nom du fichier est un prénom, ce
/// visage reçoit ce nom. Le fichier est marqué comme analysé.
pub fn enregistrer(base: &mut Connection, fichier_id: i64, chemin: &Path, visages: &[Visage]) -> Result<()> {
    let prenom = if visages.len() == 1 { prenom_du_fichier(chemin) } else { None };
    let t = base.transaction()?;
    t.execute("DELETE FROM visages WHERE fichier_id = ?1", [fichier_id])?;
    {
        let mut ajout =
            t.prepare("INSERT INTO visages (fichier_id, x, y, l, h, empreinte, prenom) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
        for v in visages {
            ajout.execute(params![fichier_id, v.boite[0], v.boite[1], v.boite[2], v.boite[3], octets(&v.empreinte), prenom])?;
        }
    }
    t.execute("UPDATE fichiers SET visages_faits = 1 WHERE id = ?1", [fichier_id])?;
    t.commit()?;
    Ok(())
}

/// Une photo où une personne cherchée apparaît.
pub struct PhotoDePersonne {
    pub chemin: String,
    /// Le prénom tel que tiré du nom de fichier.
    pub prenom: String,
    /// Ressemblance du visage (cosinus) avec un visage nommé de cette personne.
    pub ressemblance: f32,
}

/// Les photos où apparaît la personne dont le prénom correspond à `recherche`.
pub fn chercher_personne(base: &Connection, recherche: &str, max: usize) -> Result<Vec<PhotoDePersonne>> {
    let q = normaliser(recherche);
    if q.chars().count() < 3 {
        return Ok(Vec::new());
    }
    // Les visages nommés dont le prénom correspond (« elliot » trouve « elliot », « elliot martin »…).
    let mut req = base.prepare("SELECT DISTINCT prenom, empreinte FROM visages WHERE prenom IS NOT NULL")?;
    let references: Vec<(String, Vec<f32>)> = req
        .query_map([], |r| Ok((r.get::<_, String>(0)?, depuis_octets(&r.get::<_, Vec<u8>>(1)?))))?
        .filter_map(|r| r.ok())
        .filter(|(prenom, _)| prenom.split(' ').any(|m| m == q) || prenom == &q)
        .collect();
    if references.is_empty() {
        return Ok(Vec::new());
    }
    // Tous les visages, comparés aux visages nommés.
    let mut req = base.prepare(
        "SELECT f.chemin, v.empreinte FROM visages v JOIN fichiers f ON f.id = v.fichier_id WHERE f.erreur IS NULL",
    )?;
    let mut meilleurs: std::collections::HashMap<String, PhotoDePersonne> = std::collections::HashMap::new();
    for ligne in req.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
        let (chemin, octets) = ligne?;
        let e = depuis_octets(&octets);
        let (ressemblance, prenom) = references
            .iter()
            .map(|(p, r)| (r.iter().zip(&e).map(|(a, b)| a * b).sum::<f32>(), p))
            .fold((f32::MIN, &references[0].0), |m, x| if x.0 > m.0 { x } else { m });
        if ressemblance >= SEUIL_MEME_PERSONNE {
            let deja = meilleurs.get(&chemin).map_or(f32::MIN, |p| p.ressemblance);
            if ressemblance > deja {
                meilleurs.insert(chemin.clone(), PhotoDePersonne { chemin, prenom: prenom.clone(), ressemblance });
            }
        }
    }
    let mut photos: Vec<PhotoDePersonne> = meilleurs.into_values().collect();
    photos.sort_by(|a, b| b.ressemblance.total_cmp(&a.ressemblance));
    photos.truncate(max);
    Ok(photos)
}

/// Les photos pas encore analysées : (id, chemin).
pub fn photos_a_analyser(base: &Connection) -> Result<Vec<(i64, String)>> {
    let mut req = base.prepare("SELECT id, chemin FROM fichiers WHERE genre = 'photo' AND erreur IS NULL AND visages_faits = 0")?;
    let photos = req.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(photos)
}

/// Vérifie que les modèles sont là (sinon, l'analyse des visages est simplement sautée).
pub fn modeles_presents(racine: &Path) -> bool {
    let d = racine.join("modeles").join("visages");
    d.join("face_detection_yunet_2023mar.onnx").exists() && d.join("face_recognition_sface_2021dec.onnx").exists()
}

pub fn contexte(racine: &Path) -> Result<Arc<Reconnaissance>> {
    Reconnaissance::charger(racine).context("modèles de visages")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prenoms() {
        assert_eq!(prenom_du_fichier(Path::new("elliot.png")).as_deref(), Some("elliot"));
        assert_eq!(prenom_du_fichier(Path::new("Elliot Martin.jpg")).as_deref(), Some("elliot martin"));
        assert_eq!(prenom_du_fichier(Path::new("234grehrh45.png")), None);
        assert_eq!(prenom_du_fichier(Path::new("IMG_2045.jpg")), None);
        assert_eq!(prenom_du_fichier(Path::new("Capture d'écran.png")), None);
        assert_eq!(prenom_du_fichier(Path::new("photo.jpg")), None);
    }

    /// Sur un dossier de photos : visages trouvés, prénoms, et ressemblance de chaque paire.
    /// FURET_VISAGES=dossier cargo test --release -p furet-moteur sur_des_photos -- --ignored --nocapture
    #[test]
    #[ignore]
    fn sur_des_photos() {
        let dossier = std::env::var("FURET_VISAGES").expect("FURET_VISAGES");
        let r = Reconnaissance::charger(Path::new(r"D:\furet")).unwrap();
        let images = crate::photo::Images::nouveau().unwrap();
        let mut tous: Vec<(String, Vec<f32>)> = Vec::new();
        for entree in std::fs::read_dir(&dossier).unwrap().flatten() {
            let chemin = entree.path();
            let source = images.ouvrir(&chemin).unwrap();
            let p = images.pixels(&source, COTE as u32, false).unwrap();
            let t = std::time::Instant::now();
            let visages = r.analyser(&p.octets, p.largeur as usize, p.hauteur as usize).unwrap();
            let nom = chemin.file_name().unwrap().to_string_lossy().to_string();
            println!(
                "{nom:<18} {} visage(s) en {} ms, prénom tiré du nom : {:?}",
                visages.len(),
                t.elapsed().as_millis(),
                prenom_du_fichier(&chemin)
            );
            for v in visages {
                tous.push((nom.clone(), v.empreinte));
            }
        }
        for i in 0..tous.len() {
            for j in i + 1..tous.len() {
                let c: f32 = tous[i].1.iter().zip(&tous[j].1).map(|(a, b)| a * b).sum();
                let verdict = if c >= SEUIL_MEME_PERSONNE { "même personne" } else { "" };
                println!("  {:<18} / {:<18} {c:+.3} {verdict}", tous[i].0, tous[j].0);
            }
        }
    }
}
