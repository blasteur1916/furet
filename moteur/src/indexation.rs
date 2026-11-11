//! Indexation en chaîne : plusieurs fils préparent les fichiers (décodage, OCR, ffmpeg)
//! pendant que le fil principal envoie au modèle des lots qui mélangent plusieurs fichiers.

use crate::base::{self, MorceauIndexe};
use crate::fichiers::{self, Genre, Lecteurs, Preparation};
use crate::mesures::{self, SuiviVram, mo, secondes};
use crate::ocr::Ocr;
use crate::paroles;
use crate::photo::Images;
use crate::visages;
use crate::pertinence::Banque;
use crate::serveur::{Client, Serveur};
use anyhow::Result;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

/// Un lot envoyé au modèle : au plus 8 morceaux et environ 4 000 jetons.
const LOT_MORCEAUX: usize = 8;
const LOT_JETONS: usize = 3500;
/// Lots envoyés au serveur en même temps : pendant qu'il encode l'un, il décode le suivant.
const LOTS_EN_VOL: usize = 2;
/// Dossiers techniques qu'on ne parcourt pas.
const DOSSIERS_IGNORES: [&str; 14] = [
    "node_modules", "__pycache__", "venv", "$recycle.bin", "system volume information", "appdata",
    // Bibliothèques de jeux : des milliers de textures et de sons sans intérêt pour une recherche.
    "steamapps", "steamlibrary", "epic games", "riot games", "battle.net", "ubisoft game launcher", "xboxgames",
    "windowsapps",
];
/// Ignorés seulement à la racine d'un disque (un projet de code peut avoir un dossier « windows »).
const DOSSIERS_SYSTEME: [&str; 7] =
    ["windows", "program files", "program files (x86)", "programdata", "recovery", "perflogs", "$windows.~bt"];
/// En dessous, seul le nom est enregistré : sons d'interface, vidéos vides. Pas de minimum pour les images :
/// un GIF ou un schéma PNG peut peser 3 Ko ; les icônes sont écartées par leurs dimensions (fichiers.rs).
fn taille_min(genre: Genre) -> u64 {
    match genre {
        Genre::Audio => 16 << 10,
        Genre::Video => 64 << 10,
        _ => 1,
    }
}

/// Où en est l'indexation (pour l'anneau de progression de l'appli).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progres {
    /// Les dossiers sont parcourus (noms enregistrés, fichiers nouveaux ou modifiés repérés).
    Parcours,
    /// Passe 1 : les fichiers sont lus et deviennent cherchables.
    Lecture { faits: usize, total: usize },
    /// Les visages des photos sont reconnus.
    Visages { faits: usize, total: usize },
    /// Passe 2 : la parole est transcrite.
    Transcription { faits: usize, total: usize },
}

pub type Suivi = Arc<dyn Fn(Progres) + Send + Sync>;

pub struct Options {
    pub cpu: bool,
    pub tout: bool,
    pub avec_ocr: bool,
    pub avec_transcription: bool,
    /// Reconnaître les visages des photos (si les modèles sont là).
    pub avec_visages: bool,
    /// Fils de préparation (0 : automatique).
    pub fils: usize,
    /// None : 140 sur carte graphique, 70 sur processeur.
    pub jetons_image: Option<usize>,
    /// Mis à vrai, l'indexation finit les fichiers en cours puis s'arrête (pause, jeu, batterie, sortie).
    /// Ce qui est fait reste fait : la prochaine fois, elle reprend où elle en était.
    pub arret: Arc<AtomicBool>,
    pub suivi: Option<Suivi>,
}

struct Travail {
    chemin: PathBuf,
    genre: Genre,
    taille: u64,
    modifie: i64,
}

/// Ce qu'un fil de préparation renvoie pour un fichier.
struct Prepare {
    index: usize,
    resultat: Result<Preparation>,
    prep_ms: f64,
}

/// Un lot de morceaux (fichier, morceau) envoyé au modèle.
struct Lot {
    morceaux: Vec<(usize, usize)>,
    poids: usize,
}

/// Ce que reçoit le fil principal.
enum Evenement {
    Pret(Prepare),
    /// Un fil de préparation a fini (plus de fichiers, ou arrêt demandé).
    FilFini,
    Vecteurs { lot: Lot, ms: f64, resultat: Result<Vec<Vec<f32>>> },
}

/// Un fichier préparé qui attend ses vecteurs.
struct EnCours {
    prep: Preparation,
    vecteurs: Vec<Option<Vec<f32>>>,
    restants: usize,
    prep_ms: f64,
    modele_ms: f64,
}

#[derive(Default)]
struct Cumul {
    fichiers: usize,
    morceaux: usize,
    erreurs: usize,
    prep_ms: f64,
    ocr_ms: f64,
    modele_ms: f64,
    pire_ms: f64,
}

/// Dossier caché (.git, .venv…), technique (node_modules…) ou de compilation (target de Rust).
fn dossier_ignore(e: &walkdir::DirEntry) -> bool {
    if e.depth() == 0 || !e.file_type().is_dir() {
        return false;
    }
    let nom = e.file_name().to_string_lossy().to_lowercase();
    let a_la_racine = e.path().parent().is_some_and(|p| p.parent().is_none());
    // Dossiers marqués « caché » ou « système » par Windows.
    let attributs = e.metadata().map(|m| std::os::windows::fs::MetadataExt::file_attributes(&m)).unwrap_or(0);
    nom.starts_with('.')
        || attributs & 0x6 != 0
        || DOSSIERS_IGNORES.contains(&nom.as_str())
        || (a_la_racine && DOSSIERS_SYSTEME.contains(&nom.as_str()))
        || e.path().join("CACHEDIR.TAG").exists()
}

/// Indexe `dossiers` en une seule session (un seul démarrage du modèle). Les fichiers inchangés depuis la
/// dernière fois sont sautés : relancer après un arrêt reprend où on en était.
pub fn indexer(racine: &Path, chemin_base: &Path, dossiers: &[PathBuf], opt: Options) -> Result<()> {
    crate::processus::definir_racine(racine);
    let suivre = |p: Progres| {
        if let Some(s) = &opt.suivi {
            s(p);
        }
    };
    suivre(Progres::Parcours);
    let mut base = base::ouvrir(chemin_base)?;
    let mut tous: Vec<PathBuf> = Vec::new();
    let mut noms_enregistres = 0;
    for dossier in dossiers {
        if opt.arret.load(Ordering::Relaxed) {
            return Ok(());
        }
        let du_dossier: Vec<PathBuf> = walkdir::WalkDir::new(dossier)
            .into_iter()
            .filter_entry(|e| !dossier_ignore(e))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect();
        // Tous les noms, quel que soit le type : on doit pouvoir retrouver n'importe quel fichier par son nom.
        let noms: Vec<String> = du_dossier.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let prefixe = dossier.to_string_lossy();
        base::enregistrer_noms(&mut base, &prefixe, &noms)?;
        // Les fichiers supprimés ou déplacés depuis la dernière fois ne doivent plus sortir dans les résultats.
        let oublies = base::oublier_absents(&mut base, &prefixe, &noms.iter().cloned().collect())?;
        if oublies > 0 {
            println!("{oublies} fichier(s) disparu(s) retiré(s) de l'index");
        }
        noms_enregistres += noms.len();
        tous.extend(du_dossier);
    }
    let noms = noms_enregistres;
    let trouves: Vec<(PathBuf, Genre)> = tous.into_iter().filter_map(|p| Genre::de(&p).map(|g| (p, g))).collect();
    let mut par_genre: BTreeMap<Genre, usize> = BTreeMap::new();
    trouves.iter().for_each(|(_, g)| *par_genre.entry(*g).or_default() += 1);
    let resume: Vec<String> = par_genre.iter().map(|(g, n)| format!("{n} {}", g.nom())).collect();
    println!("{noms} noms de fichiers enregistrés (tous types)");
    println!("{} fichiers à lire par leur contenu : {}", trouves.len(), resume.join(", "));

    let mut travaux = Vec::new();
    let mut inchanges = 0;
    let mut minuscules = 0;
    for (chemin, genre) in trouves {
        let Ok(meta) = std::fs::metadata(&chemin) else { continue };
        let modifie = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        if meta.len() < taille_min(genre) {
            minuscules += 1;
        } else if !opt.tout && base::a_jour(&base, &chemin.to_string_lossy(), meta.len(), modifie)? {
            inchanges += 1;
        } else {
            travaux.push(Travail { chemin, genre, taille: meta.len(), modifie });
        }
    }
    if minuscules > 0 {
        println!("{minuscules} fichier(s) minuscule(s) (icônes, sons courts…) : seulement par leur nom");
    }
    if inchanges > 0 {
        println!("{inchanges} fichier(s) inchangé(s) depuis la dernière fois, ignoré(s) (--tout pour les refaire)");
    }
    let reste_a_transcrire = opt.avec_transcription && !base::a_transcrire(&base)?.is_empty();
    if travaux.is_empty() && !reste_a_transcrire {
        return Ok(());
    }

    let avec_ocr = opt.avec_ocr
        && match Ocr::nouveau() {
            Ok(o) => {
                println!("OCR : Windows, {}", o.langue());
                true
            }
            Err(e) => {
                println!("OCR désactivé : {e:#}");
                false
            }
        };
    let fils = match opt.fils {
        0 => std::thread::available_parallelism().map(|n| n.get() / 4).unwrap_or(2).clamp(2, 6),
        n => n,
    };
    println!("Préparation sur {fils} fils");

    let suivi_vram = (!opt.cpu).then(SuiviVram::demarrer);
    let serveur = Serveur::demarrer(racine, opt.cpu, opt.jetons_image, &chemin_base.with_file_name("serveur.log"))?;
    println!("Modèle chargé sur : {} (en {})\n", serveur.appareil, secondes(serveur.demarrage.as_secs_f64() * 1000.0));
    let banque = Arc::new(Banque::charger(&serveur.client())?);

    let debut = Instant::now();
    let travaux = Arc::new(travaux);
    let suivant = Arc::new(AtomicUsize::new(0));
    let pdfium = fichiers::charger_pdfium(racine)?;
    let (envoi, reception) = mpsc::sync_channel::<Evenement>(fils * 2 + LOTS_EN_VOL);
    let mut poignees = Vec::new();
    for _ in 0..fils {
        let (travaux, suivant, envoi, pdfium, arret) =
            (travaux.clone(), suivant.clone(), envoi.clone(), pdfium.clone(), opt.arret.clone());
        poignees.push(std::thread::spawn(move || {
            crate::processus::fil_en_priorite_basse();
            match Lecteurs::nouveaux(pdfium, avec_ocr) {
                Ok(lecteurs) => loop {
                    if arret.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = suivant.fetch_add(1, Ordering::Relaxed);
                    let Some(t) = travaux.get(index) else { break };
                    let debut = Instant::now();
                    let resultat = lecteurs.preparer(&t.chemin, t.genre);
                    let prep_ms = debut.elapsed().as_secs_f64() * 1000.0;
                    if envoi.send(Evenement::Pret(Prepare { index, resultat, prep_ms })).is_err() {
                        break;
                    }
                },
                Err(e) => eprintln!("fil de préparation arrêté : {e:#}"),
            }
            let _ = envoi.send(Evenement::FilFini);
        }));
    }
    // Fils d'envoi : chacun prend un lot, attend les vecteurs du serveur et les renvoie au fil principal.
    let (envoi_lots, reception_lots) = mpsc::sync_channel::<(Lot, Vec<serde_json::Value>)>(LOTS_EN_VOL);
    let reception_lots = Arc::new(Mutex::new(reception_lots));
    for _ in 0..LOTS_EN_VOL {
        let (client, reception_lots, envoi): (Client, _, SyncSender<Evenement>) =
            (serveur.client(), reception_lots.clone(), envoi.clone());
        poignees.push(std::thread::spawn(move || {
            loop {
                let suivant = reception_lots.lock().unwrap().recv();
                let Ok((lot, entrees)) = suivant else { break };
                let debut = Instant::now();
                let resultat = client.vecteurs(entrees);
                let ms = debut.elapsed().as_secs_f64() * 1000.0;
                if envoi.send(Evenement::Vecteurs { lot, ms, resultat }).is_err() {
                    break;
                }
            }
        }));
    }
    drop(envoi);

    let total = travaux.len();
    let mut chaine = Chaine {
        base,
        travaux: travaux.clone(),
        appareil: serveur.appareil.clone(),
        en_cours: HashMap::new(),
        attente: VecDeque::new(),
        poids_attente: 0,
        cumuls: BTreeMap::new(),
        termines: 0,
        prepares: 0,
        en_vol: 0,
        banque: banque.clone(),
        suivi: opt.suivi.clone(),
    };
    let mut fils_finis = 0;
    loop {
        // Plus rien ne viendra des fils de préparation : tout préparé, ou arrêt demandé.
        let preparation_finie = chaine.prepares == total || fils_finis == fils;
        if preparation_finie && chaine.termines == chaine.prepares && chaine.attente.is_empty() && chaine.en_vol == 0 {
            break;
        }
        // Remplir le serveur : des lots pleins, ou ce qui attend s'il n'a rien à faire,
        // ou tout le reste quand plus rien n'arrive des fils de préparation.
        while chaine.en_vol < LOTS_EN_VOL
            && !chaine.attente.is_empty()
            && (chaine.attente.len() >= LOT_MORCEAUX
                || chaine.poids_attente >= LOT_JETONS
                || chaine.en_vol == 0
                || preparation_finie)
        {
            let (lot, entrees) = chaine.former_lot();
            chaine.en_vol += 1;
            envoi_lots.send((lot, entrees))?;
        }
        match reception.recv() {
            Ok(Evenement::Pret(p)) => chaine.ajouter(p)?,
            Ok(Evenement::FilFini) => fils_finis += 1,
            Ok(Evenement::Vecteurs { lot, ms, resultat }) => {
                chaine.en_vol -= 1;
                chaine.recevoir(lot, ms, resultat)?;
            }
            Err(_) => break,
        }
    }
    drop(envoi_lots);
    for p in poignees {
        let _ = p.join();
    }

    let duree = debut.elapsed().as_secs_f64();
    let Chaine { mut base, cumuls, termines, .. } = chaine;
    let appareil = serveur.appareil.clone();

    println!("\n== Passe 1 : tout est cherchable ({appareil}, {fils} fils de préparation) ==");
    println!(
        "{:<8} {:>8} {:>9} {:>8} {:>15} {:>10} {:>15} {:>13}",
        "type", "fichiers", "morceaux", "erreurs", "lecture/fich.", "dont OCR", "modèle/fich.", "pire fichier"
    );
    for (g, c) in &cumuls {
        let par_fichier = |ms: f64| if c.fichiers > 0 { ms / c.fichiers as f64 } else { 0.0 };
        println!(
            "{:<8} {:>8} {:>9} {:>8} {:>15} {:>10} {:>15} {:>13}",
            g.nom(), c.fichiers, c.morceaux, c.erreurs, secondes(par_fichier(c.prep_ms)), secondes(par_fichier(c.ocr_ms)),
            secondes(par_fichier(c.modele_ms)), secondes(c.pire_ms)
        );
    }
    let debit = if duree > 0.0 { termines as f64 / duree * 60.0 } else { 0.0 };
    println!("Durée : {} pour {termines} fichier(s), soit {debit:.0} fichiers par minute", secondes(duree * 1000.0));
    println!("(la lecture se fait en parallèle sur {fils} fils : la durée est plus courte que la somme)");

    if opt.avec_visages && !opt.arret.load(Ordering::Relaxed) && visages::modeles_presents(racine) {
        let debut = Instant::now();
        match passe_visages(&mut base, racine, &opt) {
            Ok(0) => {}
            Ok(n) => println!("Durée : {} pour {n} photo(s)", secondes(debut.elapsed().as_secs_f64() * 1000.0)),
            Err(e) => println!("Visages : {e:#}"),
        }
    }

    // Les dates de prise de vue et les lieux des photos : rapide (seulement les métadonnées).
    if !opt.arret.load(Ordering::Relaxed) {
        match crate::lieux::passe(&mut base, racine, &opt.arret) {
            Ok((0, _)) => {}
            Ok((lues, placees)) => println!("\n== Lieux ==\n{lues} photo(s) lue(s), {placees} placée(s) sur la carte"),
            Err(e) => println!("Lieux : {e:#}"),
        }
    }

    let mut duree_transcription = 0.0;
    if opt.avec_transcription && !opt.arret.load(Ordering::Relaxed) {
        let journal = chemin_base.with_file_name("transcription.log");
        match paroles::transcrire(&mut base, &serveur.client(), &banque, racine, &journal, &opt.arret, opt.suivi.as_ref()) {
            Ok(Some(b)) => {
                duree_transcription = b.duree;
                let vitesse = if b.duree > 0.0 { b.secondes_audio / b.duree } else { 0.0 };
                println!(
                    "Durée : {} pour {} fichier(s) et {:.0} s de parole, soit {} fois le temps réel",
                    secondes(b.duree * 1000.0), b.fichiers, b.secondes_audio, format!("{vitesse:.1}").replace('.', ",")
                );
            }
            Ok(None) => {}
            Err(e) => println!("Transcription impossible : {e:#}"),
        }
    }

    let ram_programme = mesures::pic_memoire_programme();
    let ram_serveur = serveur.memoire_max();
    let demarrage = serveur.demarrage.as_secs_f64();
    drop(serveur);
    let vram = suivi_vram.and_then(|s| s.arreter());
    base.execute(
        "INSERT INTO sessions (date, appareil, nb_fichiers, duree_s, demarrage_s, ram_programme, ram_serveur, vram)
         VALUES (datetime('now', 'localtime'), ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![appareil, termines as i64, duree + duree_transcription, demarrage, ram_programme as i64,
                          ram_serveur as i64, vram.map(|v| v as i64)],
    )?;
    base.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;

    println!("\n== Ressources ==");
    println!("Démarrage du modèle : {}", secondes(demarrage * 1000.0));
    println!("Mémoire vive max : programme {}, llama-server {}", mo(ram_programme), mo(ram_serveur));
    if let Some(v) = vram {
        println!("Mémoire de la carte graphique : +{} pendant l'indexation", mo(v));
    }
    println!("Place de l'index : {}", mo(mesures::taille_base(chemin_base)));
    Ok(())
}

/// Reconnaît les visages des photos pas encore analysées, sur plusieurs fils (processeur, priorité basse).
fn passe_visages(base: &mut Connection, racine: &Path, opt: &Options) -> Result<usize> {
    let photos = visages::photos_a_analyser(base)?;
    if photos.is_empty() {
        return Ok(0);
    }
    let reconnaissance = visages::contexte(racine)?;
    let total = photos.len();
    println!("\n== Visages ==\n{total} photo(s) à analyser");
    let fils = std::thread::available_parallelism().map(|n| n.get() / 4).unwrap_or(2).clamp(2, 6);
    let photos = Arc::new(photos);
    let suivant = Arc::new(AtomicUsize::new(0));
    let (envoi, reception) = mpsc::channel::<(usize, Result<Vec<visages::Visage>>)>();
    for _ in 0..fils {
        let (photos, suivant, envoi, reconnaissance, arret) =
            (photos.clone(), suivant.clone(), envoi.clone(), reconnaissance.clone(), opt.arret.clone());
        std::thread::spawn(move || {
            crate::processus::fil_en_priorite_basse();
            let Ok(images) = Images::nouveau() else { return };
            loop {
                if arret.load(Ordering::Relaxed) {
                    break;
                }
                let i = suivant.fetch_add(1, Ordering::Relaxed);
                let Some((_, chemin)) = photos.get(i) else { break };
                let resultat = images
                    .ouvrir(Path::new(chemin))
                    .and_then(|s| images.pixels(&s, visages::COTE as u32, false))
                    .and_then(|p| reconnaissance.analyser(&p.octets, p.largeur as usize, p.hauteur as usize));
                if envoi.send((i, resultat)).is_err() {
                    break;
                }
            }
        });
    }
    drop(envoi);
    let mut faits = 0;
    for (i, resultat) in reception {
        let (id, chemin) = &photos[i];
        // Illisible : marquée comme faite quand même (sans visage), pour ne pas réessayer sans fin.
        visages::enregistrer(base, *id, Path::new(chemin), &resultat.unwrap_or_default())?;
        faits += 1;
        if let Some(s) = &opt.suivi {
            s(Progres::Visages { faits, total });
        }
    }
    Ok(faits)
}

/// L'état du fil principal : fichiers préparés en attente de vecteurs, et morceaux à envoyer.
struct Chaine {
    base: Connection,
    banque: Arc<Banque>,
    travaux: Arc<Vec<Travail>>,
    appareil: String,
    en_cours: HashMap<usize, EnCours>,
    /// (fichier, morceau) pas encore envoyés au modèle.
    attente: VecDeque<(usize, usize)>,
    poids_attente: usize,
    cumuls: BTreeMap<Genre, Cumul>,
    termines: usize,
    /// Fichiers reçus des fils de préparation.
    prepares: usize,
    /// Lots envoyés au serveur dont on attend les vecteurs.
    en_vol: usize,
    suivi: Option<Suivi>,
}

impl Chaine {
    fn ajouter(&mut self, p: Prepare) -> Result<()> {
        self.prepares += 1;
        match p.resultat {
            Err(e) => self.terminer_erreur(p.index, p.prep_ms, format!("{e:#}")),
            Ok(prep) => {
                let n = prep.morceaux.len();
                for (i, m) in prep.morceaux.iter().enumerate() {
                    self.attente.push_back((p.index, i));
                    self.poids_attente += m.poids;
                }
                self.en_cours.insert(
                    p.index,
                    EnCours { prep, vecteurs: vec![None; n], restants: n, prep_ms: p.prep_ms, modele_ms: 0.0 },
                );
                if n == 0 {
                    self.terminer(p.index)?;
                }
                Ok(())
            }
        }
    }

    /// Prend les premiers morceaux en attente, jusqu'à remplir un lot.
    fn former_lot(&mut self) -> (Lot, Vec<serde_json::Value>) {
        let mut morceaux = Vec::new();
        let mut poids = 0;
        while let Some(&(f, m)) = self.attente.front() {
            let p = self.en_cours[&f].prep.morceaux[m].poids;
            if !morceaux.is_empty() && (morceaux.len() >= LOT_MORCEAUX || poids + p > LOT_JETONS) {
                break;
            }
            self.attente.pop_front();
            self.poids_attente -= p;
            poids += p;
            morceaux.push((f, m));
        }
        let entrees = morceaux
            .iter()
            .map(|&(f, m)| std::mem::take(&mut self.en_cours.get_mut(&f).unwrap().prep.morceaux[m].entree))
            .collect();
        (Lot { morceaux, poids }, entrees)
    }

    /// Range les vecteurs d'un lot ; enregistre les fichiers complets.
    fn recevoir(&mut self, lot: Lot, ms: f64, resultat: Result<Vec<Vec<f32>>>) -> Result<()> {
        match resultat {
            Ok(vecteurs) => {
                let mut finis = Vec::new();
                for (&(f, m), v) in lot.morceaux.iter().zip(vecteurs) {
                    // Un fichier déjà en erreur (lot précédent) n'est plus suivi.
                    let Some(e) = self.en_cours.get_mut(&f) else { continue };
                    // Le temps du lot est partagé entre fichiers selon le poids de leurs morceaux.
                    e.modele_ms += ms * e.prep.morceaux[m].poids as f64 / lot.poids.max(1) as f64;
                    e.vecteurs[m] = Some(v);
                    e.restants -= 1;
                    if e.restants == 0 {
                        finis.push(f);
                    }
                }
                for f in finis {
                    self.terminer(f)?;
                }
            }
            Err(e) => {
                // Seuls les fichiers de ce lot échouent ; on retire leurs autres morceaux de l'attente.
                let fautifs: HashSet<usize> =
                    lot.morceaux.iter().map(|&(f, _)| f).filter(|f| self.en_cours.contains_key(f)).collect();
                let en_cours = &self.en_cours;
                let mut retire = 0;
                self.attente.retain(|&(f, m)| {
                    let garder = !fautifs.contains(&f);
                    if !garder {
                        retire += en_cours[&f].prep.morceaux[m].poids;
                    }
                    garder
                });
                self.poids_attente -= retire;
                for f in fautifs {
                    let prep_ms = self.en_cours.remove(&f).map(|e| e.prep_ms).unwrap_or(0.0);
                    self.terminer_erreur(f, prep_ms, format!("{e:#}"))?;
                }
            }
        }
        Ok(())
    }

    fn terminer(&mut self, f: usize) -> Result<()> {
        let e = self.en_cours.remove(&f).expect("fichier en cours");
        let t = &self.travaux[f];
        let chemin = t.chemin.to_string_lossy();
        let morceaux: Vec<MorceauIndexe> = e
            .prep
            .morceaux
            .iter()
            .zip(e.vecteurs)
            .map(|(m, v)| MorceauIndexe {
                partie: m.partie.clone(),
                nature: m.nature,
                debut: m.debut,
                fin: m.fin,
                vecteur: v.expect("vecteur reçu"),
                texte: m.texte.clone(),
            })
            .collect();
        let fiche = base::Fiche {
            chemin: &chemin,
            genre: t.genre.nom(),
            taille: t.taille,
            modifie: t.modifie,
            prep_ms: e.prep_ms,
            ocr_ms: e.prep.ocr_ms,
            transcription_ms: 0.0,
            modele_ms: e.modele_ms,
            pages_sans_texte: e.prep.pages_sans_texte,
            appareil: &self.appareil,
            erreur: None,
        };
        base::enregistrer(&mut self.base, &fiche, &morceaux, &self.banque)?;

        self.termines += 1;
        self.signaler();
        let scans = match e.prep.pages_sans_texte {
            0 => String::new(),
            n => format!(", {n} page(s) scannée(s)"),
        };
        let ocr = if e.prep.ocr_ms > 0.0 { format!(" dont OCR {}", secondes(e.prep.ocr_ms)) } else { String::new() };
        println!(
            "{} {:<8} lecture {}{ocr} + modèle {}, {} morceau(x){scans}  {chemin}",
            self.rang(), t.genre.nom(), secondes(e.prep_ms), secondes(e.modele_ms), morceaux.len()
        );
        let c = self.cumuls.entry(t.genre).or_default();
        c.fichiers += 1;
        c.morceaux += morceaux.len();
        c.prep_ms += e.prep_ms;
        c.ocr_ms += e.prep.ocr_ms;
        c.modele_ms += e.modele_ms;
        c.pire_ms = c.pire_ms.max(e.prep_ms + e.modele_ms);
        Ok(())
    }

    fn terminer_erreur(&mut self, f: usize, prep_ms: f64, erreur: String) -> Result<()> {
        let t = &self.travaux[f];
        let chemin = t.chemin.to_string_lossy();
        self.termines += 1;
        self.signaler();
        println!("{} {:<8} ERREUR  {chemin} : {erreur}", self.rang(), t.genre.nom());
        let fiche = base::Fiche {
            chemin: &chemin,
            genre: t.genre.nom(),
            taille: t.taille,
            modifie: t.modifie,
            prep_ms,
            ocr_ms: 0.0,
            transcription_ms: 0.0,
            modele_ms: 0.0,
            pages_sans_texte: 0,
            appareil: &self.appareil,
            erreur: Some(erreur),
        };
        base::enregistrer(&mut self.base, &fiche, &[], &self.banque)?;
        self.cumuls.entry(t.genre).or_default().erreurs += 1;
        Ok(())
    }

    fn signaler(&self) {
        if let Some(s) = &self.suivi {
            s(Progres::Lecture { faits: self.termines, total: self.travaux.len() });
        }
    }

    fn rang(&self) -> String {
        let total = self.travaux.len();
        format!("[{:>w$}/{total}]", self.termines, w = total.to_string().len())
    }
}
