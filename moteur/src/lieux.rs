//! Les lieux et les dates des photos : les coordonnées GPS et la date de prise de vue lues dans leurs
//! métadonnées (EXIF), puis la ville, la région et le pays les plus proches d'après la liste des villes
//! installée avec Furet (`modeles/lieux`, GeoNames, CC BY 4.0). Rien ne passe par internet. Portable.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDateTime, TimeZone};
use exif::{In, Tag, Value};
use rusqlite::{Connection, params};

use crate::noms::normaliser;

/// Au-delà, la photo est loin de toute ville de la liste : seuls la région et le pays sont gardés.
const VILLE_MAX_KM: f64 = 60.0;
const RAYON_TERRE_KM: f64 = 6371.0;
/// Une grande ville à moins de cette distance donne aussi son nom (banlieue de Paris → « Paris »)…
const GRANDE_VILLE_KM: f64 = 30.0;
/// … à partir de ce nombre d'habitants.
const GRANDE_VILLE_HABITANTS: u64 = 500_000;

/// Ce que disent les métadonnées d'une photo.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Metadonnees {
    /// (latitude, longitude) en degrés.
    pub gps: Option<(f64, f64)>,
    /// Prise de vue, en secondes depuis 1970 (heure locale de l'appareil).
    pub prise: Option<i64>,
}

/// Les coordonnées et la date de prise de vue d'une photo (JPEG, HEIC, TIFF, PNG, WebP, RAW).
pub fn lire_exif(chemin: &Path) -> Metadonnees {
    let Ok(f) = File::open(chemin) else { return Metadonnees::default() };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(f)) else {
        return Metadonnees::default();
    };
    let champ = |t: Tag| exif.get_field(t, In::PRIMARY);
    let degres = |valeur: Tag, sens: Tag| -> Option<f64> {
        let Value::Rational(v) = &champ(valeur)?.value else { return None };
        let d = v.first()?.to_f64()
            + v.get(1).map_or(0.0, |m| m.to_f64() / 60.0)
            + v.get(2).map_or(0.0, |s| s.to_f64() / 3600.0);
        let sens = champ(sens).map(|f| f.display_value().to_string()).unwrap_or_default();
        Some(if sens.contains('S') || sens.contains('W') { -d } else { d })
    };
    let gps = match (degres(Tag::GPSLatitude, Tag::GPSLatitudeRef), degres(Tag::GPSLongitude, Tag::GPSLongitudeRef)) {
        // (0, 0) : un appareil sans position qui écrit des zéros.
        (Some(la), Some(lo)) if la.abs() <= 90.0 && lo.abs() <= 180.0 && (la, lo) != (0.0, 0.0) => Some((la, lo)),
        _ => None,
    };
    let prise = [Tag::DateTimeOriginal, Tag::DateTimeDigitized].into_iter().find_map(|t| {
        let Value::Ascii(v) = &champ(t)?.value else { return None };
        let texte = std::str::from_utf8(v.first()?).ok()?;
        let d = NaiveDateTime::parse_from_str(texte.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
        Local.from_local_datetime(&d).earliest().map(|t| t.timestamp()).filter(|&t| t > 0)
    });
    Metadonnees { gps, prise }
}

struct Ville {
    lat: f64,
    lon: f64,
    nom: String,
    region: String,
    pays: String,
    habitants: u64,
    /// Ses autres noms (« Barcelone », « Londres »), séparés par |.
    autres: String,
}

/// La liste des villes du monde (plus de 15 000 habitants) et les noms des pays.
pub struct Villes {
    villes: Vec<Ville>,
    /// Code du pays → (nom en français, autres noms séparés par |).
    pays: HashMap<String, (String, String)>,
}

fn dossier(racine: &Path) -> PathBuf {
    racine.join("modeles").join("lieux")
}

impl Villes {
    pub fn presentes(racine: &Path) -> bool {
        dossier(racine).join("villes.tsv").exists() && dossier(racine).join("pays.tsv").exists()
    }

    pub fn charger(racine: &Path) -> Result<Self> {
        let d = dossier(racine);
        let texte = std::fs::read_to_string(d.join("villes.tsv")).context("liste des villes")?;
        let villes = texte
            .lines()
            .filter_map(|l| {
                let mut c = l.split('\t');
                Some(Ville {
                    lat: c.next()?.parse().ok()?,
                    lon: c.next()?.parse().ok()?,
                    nom: c.next()?.to_owned(),
                    region: c.next()?.to_owned(),
                    pays: c.next()?.to_owned(),
                    habitants: c.next()?.parse().unwrap_or(0),
                    autres: c.next().unwrap_or("").to_owned(),
                })
            })
            .collect();
        let texte = std::fs::read_to_string(d.join("pays.tsv")).context("liste des pays")?;
        let pays = texte
            .lines()
            .filter_map(|l| {
                let mut c = l.split('\t');
                Some((c.next()?.to_owned(), (c.next()?.to_owned(), c.next().unwrap_or("").to_owned())))
            })
            .collect();
        Ok(Villes { villes, pays })
    }

    /// La ville la plus proche et sa distance en km. D'abord une distance approchée (rapide) pour toutes,
    /// puis la vraie pour la meilleure.
    fn plus_proche(&self, lat: f64, lon: f64) -> Option<(&Ville, f64)> {
        let c = lat.to_radians().cos();
        let approchee = |v: &Ville| {
            let dlon = (v.lon - lon + 540.0).rem_euclid(360.0) - 180.0;
            (v.lat - lat).powi(2) + (dlon * c).powi(2)
        };
        let v = self.villes.iter().min_by(|a, b| approchee(a).total_cmp(&approchee(b)))?;
        let d = distance_km(lat, lon, v.lat, v.lon);
        // Un arrondissement (« Paris 04 Hôtel-de-Ville ») est dans sa ville : bien plus peuplée, tout près.
        let rayon = (2.0 * d).max(5.0);
        let englobante = self
            .villes
            .iter()
            .filter(|w| w.habitants >= 5 * v.habitants.max(1) && distance_km(lat, lon, w.lat, w.lon) <= rayon)
            .max_by_key(|w| w.habitants);
        Some(match englobante {
            Some(w) => (w, distance_km(lat, lon, w.lat, w.lon)),
            None => (v, d),
        })
    }

    /// La plus grande ville proche, si elle est vraiment grande (et n'est pas `sauf`).
    fn grande_ville_proche(&self, lat: f64, lon: f64, sauf: &str) -> Option<&Ville> {
        self.villes
            .iter()
            .filter(|v| v.habitants >= GRANDE_VILLE_HABITANTS && v.nom != sauf)
            .filter(|v| distance_km(lat, lon, v.lat, v.lon) <= GRANDE_VILLE_KM)
            .max_by_key(|v| v.habitants)
    }
}

/// Met en forme `modeles/lieux` à partir des fichiers téléchargés de GeoNames (`cities15000.zip`,
/// `admin1CodesASCII.txt`, `countryInfo.txt`) et du CLDR (`territories_fr.json`) :
/// villes.tsv (lat, lon, nom, région, code pays, habitants, autres noms) et pays.tsv (code, nom, autres noms).
pub fn preparer(source: &Path, sortie: &Path) -> Result<()> {
    use std::fmt::Write as _;
    fs_creer(sortie)?;
    // Un nom en écriture latine, qui commence par une majuscule (pas « BCN », ni « lndn »).
    let vrai_nom = |n: &str| {
        let n = n.trim();
        (3..=40).contains(&n.chars().count())
            && n.chars().next().is_some_and(char::is_uppercase)
            && n.chars().any(char::is_lowercase)
            && n.chars().all(|c| " -'’.()".contains(c) || (c.is_alphabetic() && (c as u32) < 0x250))
    };
    let regions: HashMap<String, String> = std::fs::read_to_string(source.join("admin1CodesASCII.txt"))?
        .lines()
        .filter_map(|l| {
            let mut c = l.split('\t');
            Some((c.next()?.to_owned(), c.next()?.to_owned()))
        })
        .collect();
    let mut zip = zip::ZipArchive::new(File::open(source.join("cities15000.zip"))?)?;
    let mut texte = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("cities15000.txt")?, &mut texte)?;
    let mut villes = String::with_capacity(texte.len() / 2);
    for l in texte.lines() {
        let c: Vec<&str> = l.split('\t').collect();
        // Les quartiers d'une ville (PPLX) ne sont pas des villes : « Dreta de l'Eixample » est à Barcelone.
        if c.len() < 15 || c[7] == "PPLX" {
            continue;
        }
        let (nom, ascii, autres, pays) = (c[1], c[2], c[3], c[8]);
        let mut vus: HashSet<String> = HashSet::from([nom.to_lowercase()]);
        let noms: Vec<&str> = std::iter::once(ascii)
            .chain(autres.split(','))
            .map(str::trim)
            .filter(|a| vrai_nom(a) && vus.insert(a.to_lowercase()))
            .collect();
        let region = regions.get(&format!("{pays}.{}", c[10])).map_or("", String::as_str);
        let (lat, lon): (f64, f64) = (c[4].parse().unwrap_or(0.0), c[5].parse().unwrap_or(0.0));
        let _ = writeln!(villes, "{lat:.4}\t{lon:.4}\t{nom}\t{region}\t{pays}\t{}\t{}", c[14], noms.join("|"));
    }
    let francais: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(source.join("territories_fr.json"))?)?;
    let francais = &francais["main"]["fr"]["localeDisplayNames"]["territories"];
    let mut pays = String::new();
    for l in std::fs::read_to_string(source.join("countryInfo.txt"))?.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = l.split('\t').collect();
        if c.len() < 5 {
            continue;
        }
        let (code, anglais) = (c[0], c[4]);
        let fr = francais[code].as_str().unwrap_or(anglais);
        let mut autres: Vec<&str> = [Some(anglais), francais[format!("{code}-alt-short")].as_str(), francais[format!("{code}-alt-variant")].as_str()]
            .into_iter()
            .flatten()
            .filter(|n| *n != fr)
            .collect();
        autres.sort();
        autres.dedup();
        let _ = writeln!(pays, "{code}\t{fr}\t{}", autres.join("|"));
    }
    if villes.len() < 1000 || pays.is_empty() {
        anyhow::bail!("liste des villes incomplète");
    }
    std::fs::write(sortie.join("pays.tsv"), pays)?;
    std::fs::write(
        sortie.join("LISEZMOI.txt"),
        "Villes du monde de plus de 15 000 habitants : GeoNames (https://www.geonames.org), licence CC BY 4.0.\n\
         Noms des pays en français : Unicode CLDR (https://cldr.unicode.org), licence Unicode.\n",
    )?;
    // En dernier : villes.tsv prouve que tout est là.
    std::fs::write(sortie.join("villes.tsv"), villes)?;
    Ok(())
}

fn fs_creer(d: &Path) -> Result<()> {
    std::fs::create_dir_all(d).with_context(|| format!("création de {}", d.display()))
}

fn distance_km(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let (p1, p2) = (la1.to_radians(), la2.to_radians());
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lo2 - lo1).to_radians() / 2.0).sin().powi(2);
    2.0 * RAYON_TERRE_KM * a.sqrt().asin()
}

/// « Barcelona|Barcelone|Barcellona » → « barcelona|barcelone|barcellona »
fn noms_normalises<'a>(noms: impl IntoIterator<Item = &'a str>) -> String {
    let mut vus = HashSet::new();
    noms.into_iter()
        .flat_map(|n| n.split('|'))
        .map(normaliser)
        .filter(|n| n.len() >= 3 && vus.insert(n.clone()))
        .collect::<Vec<_>>()
        .join("|")
}

/// Lit les métadonnées des photos qui ne l'ont pas encore été, puis place sur la carte celles qui ont des
/// coordonnées. Rend (photos lues, photos placées).
pub fn passe(base: &mut Connection, racine: &Path, arret: &AtomicBool) -> Result<(usize, usize)> {
    let photos: Vec<(i64, String)> = {
        let mut req =
            base.prepare("SELECT id, chemin FROM fichiers WHERE genre = 'photo' AND erreur IS NULL AND exif_fait = 0")?;
        req.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    // Lire les métadonnées ne lit que le début du fichier : quelques fils suffisent.
    let fils = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 4);
    let lues: Vec<(i64, Metadonnees)> = std::thread::scope(|s| {
        let parts: Vec<_> = photos
            .chunks(photos.len().div_ceil(fils).max(1))
            .map(|lot| {
                s.spawn(move || {
                    crate::processus::fil_en_priorite_basse();
                    lot.iter()
                        .take_while(|_| !arret.load(Ordering::Relaxed))
                        .map(|(id, chemin)| (*id, lire_exif(Path::new(chemin))))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        parts.into_iter().flat_map(|p| p.join().unwrap_or_default()).collect()
    });
    let t = base.transaction()?;
    {
        let mut maj = t.prepare("UPDATE fichiers SET lat = ?2, lon = ?3, prise = ?4, exif_fait = 1 WHERE id = ?1")?;
        for (id, m) in &lues {
            maj.execute(params![id, m.gps.map(|g| g.0), m.gps.map(|g| g.1), m.prise])?;
        }
    }
    t.commit()?;

    let a_placer: Vec<(i64, f64, f64)> = {
        let mut req = base.prepare("SELECT id, lat, lon FROM fichiers WHERE lat IS NOT NULL AND lieu_id IS NULL")?;
        req.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
    };
    if a_placer.is_empty() || !Villes::presentes(racine) {
        return Ok((lues.len(), 0));
    }
    let villes = Villes::charger(racine)?;
    let t = base.transaction()?;
    let mut lieux: HashMap<(String, String, String), i64> = HashMap::new();
    {
        let mut ajout = t.prepare(
            "INSERT OR IGNORE INTO lieux (ville, region, pays, noms_ville, noms_region, noms_pays) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut cherche = t.prepare("SELECT id FROM lieux WHERE ville = ?1 AND region = ?2 AND pays = ?3")?;
        let mut maj = t.prepare("UPDATE fichiers SET lieu_id = ?2 WHERE id = ?1")?;
        for (id, lat, lon) in &a_placer {
            if arret.load(Ordering::Relaxed) {
                break;
            }
            let Some((v, km)) = villes.plus_proche(*lat, *lon) else { continue };
            let (pays, autres_pays) = villes.pays.get(&v.pays).cloned().unwrap_or_else(|| (v.pays.clone(), String::new()));
            let (ville, autres_ville) = if km <= VILLE_MAX_KM { (v.nom.clone(), v.autres.as_str()) } else { (String::new(), "") };
            let grande = villes.grande_ville_proche(*lat, *lon, &ville);
            let cle = (ville.clone(), v.region.clone(), pays.clone());
            let lieu = match lieux.get(&cle) {
                Some(l) => *l,
                None => {
                    ajout.execute(params![
                        ville,
                        v.region,
                        pays,
                        noms_normalises([
                            ville.as_str(),
                            autres_ville,
                            grande.map_or("", |g| g.nom.as_str()),
                            grande.map_or("", |g| g.autres.as_str()),
                        ]),
                        noms_normalises([v.region.as_str()]),
                        noms_normalises([pays.as_str(), autres_pays.as_str(), v.pays.as_str()]),
                    ])?;
                    let l: i64 = cherche.query_row(params![ville, v.region, pays], |r| r.get(0))?;
                    lieux.insert(cle, l);
                    l
                }
            };
            maj.execute(params![id, lieu])?;
        }
    }
    t.commit()?;
    Ok((lues.len(), a_placer.len()))
}

/// Un lieu nommé dans une recherche (« vacances Espagne ») où l'utilisateur a pris des photos.
#[derive(Debug, Clone)]
pub struct LieuTrouve {
    /// « Espagne », « Barcelona, Espagne »
    pub etiquette: String,
    /// Les lieux de l'index qui portent ce nom.
    pub ids: Vec<i64>,
    /// La recherche sans le nom du lieu (normalisée) : « vacances ».
    pub reste: String,
}

/// Cherche dans `texte` le nom d'un lieu où des photos de l'index ont été prises : un pays, une région ou
/// une ville (en plusieurs mots aussi : « New York »). Seuls ces lieux comptent : « orange » n'est un lieu
/// que pour qui a des photos prises à Orange.
pub fn reperer(base: &Connection, texte: &str) -> Result<Option<LieuTrouve>> {
    let mots: Vec<String> = normaliser(texte).split(' ').filter(|m| !m.is_empty()).map(String::from).collect();
    if mots.is_empty() {
        return Ok(None);
    }
    struct Lieu {
        id: i64,
        noms: [String; 3],
        affichage: [String; 3],
    }
    let lieux: Vec<Lieu> = {
        let mut req = base.prepare("SELECT id, ville, region, pays, noms_ville, noms_region, noms_pays FROM lieux")?;
        req.query_map([], |r| {
            let (ville, region, pays): (String, String, String) = (r.get(1)?, r.get(2)?, r.get(3)?);
            Ok(Lieu {
                id: r.get(0)?,
                noms: [r.get(4)?, r.get(5)?, r.get(6)?],
                affichage: [
                    if ville.is_empty() { pays.clone() } else { format!("{ville}, {pays}") },
                    if region.is_empty() { pays.clone() } else { format!("{region}, {pays}") },
                    pays,
                ],
            })
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    if lieux.is_empty() {
        return Ok(None);
    }
    // Les noms les plus longs d'abord (« new york » avant « york ») ; à longueur égale, pays, région, ville.
    for n in (1..=3.min(mots.len())).rev() {
        for i in 0..=mots.len() - n {
            let groupe = mots[i..i + n].join(" ");
            if groupe.len() < 3 {
                continue;
            }
            for niveau in [2, 1, 0] {
                let trouves: Vec<&Lieu> =
                    lieux.iter().filter(|l| l.noms[niveau].split('|').any(|x| x == groupe)).collect();
                let Some(premier) = trouves.first() else { continue };
                let ids = trouves.iter().map(|l| l.id).collect();
                let reste = mots[..i].iter().chain(&mots[i + n..]).cloned().collect::<Vec<_>>().join(" ");
                return Ok(Some(LieuTrouve { etiquette: premier.affichage[niveau].clone(), ids, reste }));
            }
        }
    }
    Ok(None)
}

/// Les photos prises au lieu trouvé.
pub fn photos(base: &Connection, lieu: &LieuTrouve) -> Result<HashSet<String>> {
    let ids = lieu.ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let mut req = base.prepare(&format!("SELECT chemin FROM fichiers WHERE erreur IS NULL AND lieu_id IN ({ids})"))?;
    let photos = req.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(photos)
}

/// « Barcelona, Espagne » pour une photo placée sur la carte.
pub fn lieu_de(base: &Connection, chemin: &str) -> Result<Option<String>> {
    let lieu: Option<(String, String, String)> = base
        .query_row(
            "SELECT l.ville, l.region, l.pays FROM fichiers f JOIN lieux l ON l.id = f.lieu_id WHERE f.chemin = ?1",
            [chemin],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    Ok(lieu.map(|(ville, region, pays)| {
        let premier = if ville.is_empty() { region } else { ville };
        if premier.is_empty() { pays } else { format!("{premier}, {pays}") }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        // Paris → Londres : ~344 km.
        let d = distance_km(48.8534, 2.3488, 51.5085, -0.1257);
        assert!((d - 344.0).abs() < 5.0, "{d}");
    }

    /// Sur la vraie liste : cargo test -p furet-moteur --lib lieux -- --ignored
    #[test]
    #[ignore]
    fn villes_proches() {
        let villes = Villes::charger(Path::new(r"D:\furet")).unwrap();
        let (v, km) = villes.plus_proche(41.40, 2.17).unwrap();
        assert_eq!((v.nom.as_str(), v.pays.as_str()), ("Barcelona", "ES"), "{km}");
        let (v, _) = villes.plus_proche(48.86, 2.35).unwrap();
        assert_eq!(v.nom, "Paris");
        assert_eq!(villes.pays["ES"].0, "Espagne");
        let n = noms_normalises([v.nom.as_str(), v.autres.as_str()]);
        assert!(n.split('|').any(|x| x == "paris"));
    }
}
