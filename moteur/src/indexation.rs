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
use std::sync::{Arc, Condvar, Mutex};
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
/// Un dossier qui contient plus de fichiers de code que ça (sans compter ses projets à part, voir
/// `bibliotheques`) est une bibliothèque : moteur de jeu, SDK, dépendances. Son code n'est cherché que par
/// son nom. Mesuré sur le disque de l'utilisateur (11 novembre 2026) : 372 000 fichiers de code sur 490 000
/// fichiers lisibles, dont 320 000 dans deux dossiers, soit plus de 20 heures de carte graphique.
const BIBLIOTHEQUE_FICHIERS: usize = 3000;
/// Un fichier en erreur et inchangé n'est réessayé qu'après ce délai (un format illisible le reste).
const REESSAI_ERREUR_S: i64 = 7 * 86_400;

/// L'ordre de lecture : ce que l'utilisateur a fait ou rangé lui-même d'abord, le code à la fin.
fn priorite(genre: Genre) -> u8 {
    match genre {
        Genre::Document | Genre::Pdf | Genre::Tableur => 0,
        Genre::Photo => 1,
        Genre::Video | Genre::Audio => 2,
        Genre::Texte => 3,
        Genre::Archive => 4,
        Genre::Code => 5,
    }
}

/// Du code, ou un fichier de réglages ou de données que seul un programme lit.
fn de_bibliotheque(t: &Travail) -> bool {
    let ext = t.chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    t.genre == Genre::Code || (t.genre == Genre::Texte && !matches!(ext.as_str(), "md" | "markdown" | "txt" | "rst" | "eml"))
}

/// Les dossiers qui sont des bibliothèques de code, d'après le parcours : (dossier, fichiers de code
/// directement dedans, contient un `.git`). On compte de bas en haut ; un dossier qui atteint le seuil est
/// une bibliothèque, et ne pèse plus sur ses parents (un disque qui contient un moteur de jeu n'est pas une
/// bibliothèque). Un projet à part (avec son `.git`) sous le seuil ne compte pas non plus pour ses parents,
/// et reste lu même dans une bibliothèque. Rend, pour chaque dossier, s'il est dans une bibliothèque.
fn bibliotheques(racine: &Path, dossiers: &[(PathBuf, usize, bool)]) -> HashMap<PathBuf, bool> {
    let mut totaux: HashMap<&Path, usize> = dossiers.iter().map(|(d, n, _)| (d.as_path(), *n)).collect();
    let mut ordre: Vec<&(PathBuf, usize, bool)> = dossiers.iter().collect();
    ordre.sort_by_key(|(d, _, _)| std::cmp::Reverse(d.components().count()));
    // Pour chaque dossier : Some(vrai) bibliothèque, Some(faux) projet à part, None ni l'un ni l'autre.
    let mut sorte: HashMap<&Path, Option<bool>> = HashMap::new();
    for (d, _, git) in ordre {
        let total = totaux[d.as_path()];
        let s = if d == racine {
            None
        } else if total >= BIBLIOTHEQUE_FICHIERS {
            Some(true)
        } else if *git {
            Some(false)
        } else {
            None
        };
        sorte.insert(d.as_path(), s);
        if s.is_none()
            && let Some(parent) = d.parent()
            && let Some(t) = totaux.get_mut(parent)
        {
            *t += total;
        }
    }
    dossiers
        .iter()
        .map(|(d, _, _)| {
            // On remonte jusqu'à la première bibliothèque ou au premier projet à part.
            let dedans = d.ancestors().find_map(|a| sorte.get(a).copied().flatten()).unwrap_or(false);
            (d.clone(), dedans)
        })
        .collect()
}

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
    /// Lire aussi le code des grandes bibliothèques (moteurs de jeu, SDK…) au lieu de ne garder que leurs noms.
    pub bibliotheques: bool,
    /// Enregistrer aussi tous les noms de fichiers dans l'index (le prototype cherche les noms là ; l'appli,
    /// elle, a la liste des disques : inutile de réécrire un million de noms à chaque passage).
    pub noms: bool,
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

/// Dossier caché (.git, .venv…), technique (node_modules…) ou de système. `a_la_racine` : il est
/// directement à la racine d'un disque.
fn dossier_ignore(e: &std::fs::DirEntry, a_la_racine: bool) -> bool {
    let nom = e.file_name().to_string_lossy().to_lowercase();
    // Dossiers marqués « caché » ou « système » par Windows (connu sans relire le disque).
    #[cfg(windows)]
    let cache = e.metadata().is_ok_and(|m| std::os::windows::fs::MetadataExt::file_attributes(&m) & 0x6 != 0);
    #[cfg(not(windows))]
    let cache = false;
    nom.starts_with('.')
        || cache
        || DOSSIERS_IGNORES.contains(&nom.as_str())
        || (a_la_racine && DOSSIERS_SYSTEME.contains(&nom.as_str()))
}

/// Ce que le parcours d'un dossier a trouvé.
#[derive(Default)]
struct Liste {
    /// Les fichiers dont le type se lit, avec leur taille et leur date.
    fichiers: Vec<Travail>,
    /// Tous les chemins, quel que soit le type (seulement si demandé).
    noms: Vec<String>,
    nb_noms: usize,
    /// Dossiers qui n'ont pas pu être lus (accès refusé, erreur passagère) : on ne sait pas ce qu'ils
    /// contiennent, donc on n'oublie rien de ce qui s'y trouvait.
    illisibles: Vec<PathBuf>,
    /// Chaque dossier parcouru : (chemin, fichiers de code directement dedans, contient un `.git`).
    dossiers: Vec<(PathBuf, usize, bool)>,
}

/// Parcourt `dossier` sur plusieurs fils. La taille et la date viennent avec la liste de chaque dossier
/// (Windows les donne en même temps que les noms) : aucun fichier n'est ouvert. Mesures dans RECHERCHE.md.
fn lister(dossier: &Path, avec_noms: bool, arret: &AtomicBool) -> Liste {
    struct Partage {
        pile: Vec<PathBuf>,
        actifs: usize,
    }
    let partage = Mutex::new(Partage { pile: vec![dossier.to_path_buf()], actifs: 0 });
    let reveil = Condvar::new();
    let fils = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let locales: Vec<Liste> = std::thread::scope(|s| {
        let poignees: Vec<_> = (0..fils)
            .map(|_| {
                s.spawn(|| {
                    crate::processus::fil_en_priorite_basse();
                    let mut liste = Liste::default();
                    loop {
                        let chemin = {
                            let mut p = partage.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if arret.load(Ordering::Relaxed) || (p.pile.is_empty() && p.actifs == 0) {
                                    reveil.notify_all();
                                    return liste;
                                }
                                if let Some(d) = p.pile.pop() {
                                    p.actifs += 1;
                                    break d;
                                }
                                p = reveil.wait(p).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        let mut sous_dossiers = Vec::new();
                        match std::fs::read_dir(&chemin) {
                            Err(_) => liste.illisibles.push(chemin.clone()),
                            Ok(entrees) => {
                                let a_la_racine = chemin.parent().is_none();
                                let (mut fichiers, mut noms, mut nb_noms) = (Vec::new(), Vec::new(), 0);
                                let (mut code, mut git) = (0, false);
                                // Un dossier de cache (norme CACHEDIR.TAG) est ignoré en entier, sauf celui demandé.
                                let mut cache = false;
                                for e in entrees.flatten() {
                                    let Ok(sorte) = e.file_type() else { continue };
                                    if e.file_name() == ".git" {
                                        git = true;
                                    }
                                    if sorte.is_dir() {
                                        if !dossier_ignore(&e, a_la_racine) {
                                            sous_dossiers.push(e.path());
                                        }
                                    } else if sorte.is_file() {
                                        if e.file_name() == "CACHEDIR.TAG" && chemin != dossier {
                                            cache = true;
                                            break;
                                        }
                                        nb_noms += 1;
                                        let chemin_fichier = e.path();
                                        if avec_noms {
                                            noms.push(chemin_fichier.to_string_lossy().into_owned());
                                        }
                                        let Some(genre) = Genre::de(&chemin_fichier) else { continue };
                                        code += usize::from(genre == Genre::Code);
                                        let Ok(infos) = e.metadata() else { continue };
                                        let modifie = infos
                                            .modified()
                                            .ok()
                                            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                                            .map_or(0, |d| d.as_secs() as i64);
                                        fichiers.push(Travail { chemin: chemin_fichier, genre, taille: infos.len(), modifie });
                                    }
                                }
                                if cache {
                                    sous_dossiers.clear();
                                } else {
                                    liste.fichiers.append(&mut fichiers);
                                    liste.noms.append(&mut noms);
                                    liste.nb_noms += nb_noms;
                                    liste.dossiers.push((chemin.clone(), code, git));
                                }
                            }
                        }
                        let mut p = partage.lock().unwrap_or_else(|e| e.into_inner());
                        p.pile.append(&mut sous_dossiers);
                        p.actifs -= 1;
                        reveil.notify_all();
                    }
                })
            })
            .collect();
        poignees.into_iter().filter_map(|p| p.join().ok()).collect()
    });
    let mut tout = Liste::default();
    for mut l in locales {
        tout.fichiers.append(&mut l.fichiers);
        tout.noms.append(&mut l.noms);
        tout.nb_noms += l.nb_noms;
        tout.illisibles.append(&mut l.illisibles);
        tout.dossiers.append(&mut l.dossiers);
    }
    // Toujours le même ordre, d'un passage à l'autre.
    tout.fichiers.sort_by(|a, b| a.chemin.cmp(&b.chemin));
    tout
}

/// Le parcours d'un dossier, pour les essais : (chemin, type, taille) des fichiers dont le type se lit.
/// Le dernier élément : le fichier est du code d'une grande bibliothèque (cherché par son nom seulement).
pub fn parcours_detaille(dossier: &Path) -> Vec<(PathBuf, &'static str, u64, bool)> {
    let liste = lister(dossier, false, &AtomicBool::new(false));
    let dans = bibliotheques(dossier, &liste.dossiers);
    liste
        .fichiers
        .into_iter()
        .map(|t| {
            let b = de_bibliotheque(&t) && t.chemin.parent().is_some_and(|p| dans.get(p).copied().unwrap_or(false));
            (t.chemin, t.genre.nom(), t.taille, b)
        })
        .collect()
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
    if !opt.noms {
        base::vider_noms(&base)?;
    }
    // L'état de tout l'index, lu une fois : pour sauter les fichiers inchangés et oublier ceux qui ont disparu.
    let etats = base::etats(&base)?;
    let mut trouves: Vec<Travail> = Vec::new();
    let mut noms = 0;
    let mut de_bibliotheques = 0;
    for dossier in dossiers {
        // Disque débranché, dossier renommé : on ne touche à rien (sinon tout son index serait oublié).
        if !dossier.is_dir() {
            println!("Dossier introuvable pour l'instant, laissé tel quel : {}", dossier.display());
            continue;
        }
        let liste = lister(dossier, opt.noms, &opt.arret);
        // Arrêté en cours de route : la liste est incomplète, on ne peut rien en conclure.
        if opt.arret.load(Ordering::Relaxed) {
            return Ok(());
        }
        let prefixe = dossier.to_string_lossy();
        if opt.noms {
            // Tous les noms, quel que soit le type : le prototype retrouve ainsi n'importe quel fichier par son nom.
            base::enregistrer_noms(&mut base, &prefixe, &liste.noms)?;
        }
        // Les fichiers supprimés ou déplacés depuis la dernière fois ne doivent plus sortir dans les résultats
        // (sauf ceux d'un dossier qui n'a pas pu être lu cette fois-ci).
        let presents: HashSet<&Path> = liste.fichiers.iter().map(|t| t.chemin.as_path()).collect();
        let disparus: Vec<i64> = etats
            .iter()
            .filter(|(chemin, _)| chemin.starts_with(prefixe.as_ref()))
            .filter(|(chemin, _)| {
                let chemin = Path::new(chemin.as_str());
                !presents.contains(chemin) && !liste.illisibles.iter().any(|d| chemin.starts_with(d))
            })
            .map(|(_, etat)| etat.id)
            .collect();
        if !disparus.is_empty() {
            base::oublier(&mut base, &disparus)?;
            println!("{} fichier(s) disparu(s) retiré(s) de l'index", disparus.len());
        }
        noms += liste.nb_noms;
        // Le code des grandes bibliothèques : par son nom seulement. Ce qu'une version précédente en avait lu
        // sort de l'index (il noyait les vrais résultats sous du code de moteur de jeu).
        let dans_bibliotheque = if opt.bibliotheques { HashMap::new() } else { bibliotheques(dossier, &liste.dossiers) };
        let mut a_oublier = Vec::new();
        for t in liste.fichiers {
            let en_bibliotheque =
                de_bibliotheque(&t) && t.chemin.parent().is_some_and(|p| dans_bibliotheque.get(p).copied().unwrap_or(false));
            if en_bibliotheque {
                de_bibliotheques += 1;
                if let Some(e) = etats.get(t.chemin.to_string_lossy().as_ref()) {
                    a_oublier.push(e.id);
                }
            } else {
                trouves.push(t);
            }
        }
        if !a_oublier.is_empty() {
            println!("{} fichier(s) de bibliothèques de code retiré(s) de l'index (toujours trouvables par leur nom)", a_oublier.len());
            // Par paquets : l'appli peut lire l'index entre deux, et un arrêt garde ce qui est fait.
            for paquet in a_oublier.chunks(2000) {
                if opt.arret.load(Ordering::Relaxed) {
                    return Ok(());
                }
                base::oublier(&mut base, paquet)?;
            }
        }
    }
    if de_bibliotheques > 0 {
        println!("{de_bibliotheques} fichier(s) de grandes bibliothèques de code : seulement par leur nom");
    }
    let mut par_genre: BTreeMap<Genre, usize> = BTreeMap::new();
    trouves.iter().for_each(|t| *par_genre.entry(t.genre).or_default() += 1);
    let resume: Vec<String> = par_genre.iter().map(|(g, n)| format!("{n} {}", g.nom())).collect();
    println!("{noms} noms de fichiers vus (tous types)");
    println!("{} fichiers à lire par leur contenu : {}", trouves.len(), resume.join(", "));

    let mut travaux = Vec::new();
    let mut inchanges = 0;
    let mut minuscules = 0;
    let mut illisibles = 0;
    let maintenant = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    for t in trouves {
        let etat = etats.get(t.chemin.to_string_lossy().as_ref()).filter(|e| e.taille == t.taille as i64 && e.modifie == t.modifie);
        if t.taille < taille_min(t.genre) {
            minuscules += 1;
        } else if opt.tout {
            travaux.push(t);
        } else if etat.is_some_and(|e| e.sans_erreur) {
            inchanges += 1;
        } else if etat.is_some_and(|e| e.essai.is_some_and(|quand| maintenant - quand < REESSAI_ERREUR_S)) {
            // Illisible la dernière fois et inchangé depuis : inutile de réessayer à chaque passage.
            illisibles += 1;
        } else {
            travaux.push(t);
        }
    }
    drop(etats);
    if illisibles > 0 {
        println!("{illisibles} fichier(s) illisible(s) la dernière fois et inchangé(s) : pas réessayé(s) avant quelques jours");
    }
    // Les documents, les photos et les vidéos d'abord ; le code à la fin (l'ordre des chemins est gardé dans chaque groupe).
    travaux.sort_by_key(|t| priorite(t.genre));
    if minuscules > 0 {
        println!("{minuscules} fichier(s) minuscule(s) (icônes, sons courts…) : seulement par leur nom");
    }
    if inchanges > 0 {
        println!("{inchanges} fichier(s) inchangé(s) depuis la dernière fois, ignoré(s) (--tout pour les refaire)");
    }
    let reste_a_transcrire = opt.avec_transcription && !base::a_transcrire(&base)?.is_empty();
    if travaux.is_empty() && !reste_a_transcrire {
        // Rien à lire : pas besoin du modèle. Les visages et les lieux en retard (option activée après coup)
        // se font quand même.
        if opt.avec_visages && visages::modeles_presents(racine) {
            if let Err(e) = passe_visages(&mut base, racine, &opt) {
                println!("Visages : {e:#}");
            }
        }
        if !opt.arret.load(Ordering::Relaxed) {
            if let Err(e) = crate::lieux::passe(&mut base, racine, &opt.arret) {
                println!("Lieux : {e:#}");
            }
        }
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
                let mut restants = 0;
                // Les morceaux sans entrée (texte seul, dans les très gros fichiers) n'attendent pas le modèle.
                for (i, m) in prep.morceaux.iter().enumerate().filter(|(_, m)| !m.entree.is_null()) {
                    self.attente.push_back((p.index, i));
                    self.poids_attente += m.poids;
                    restants += 1;
                }
                self.en_cours.insert(
                    p.index,
                    EnCours { prep, vecteurs: vec![None; n], restants, prep_ms: p.prep_ms, modele_ms: 0.0 },
                );
                if restants == 0 {
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
                vecteur: v,
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
