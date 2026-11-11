//! Les noms de tous les fichiers des disques, comme Everything, sans droits administrateur.
//!
//! - Parcours : plusieurs fils lisent les dossiers avec `FindFirstFileExW` (infos de base, gros lots),
//!   la lecture de dossier la plus rapide qu'offre Windows sans lire la MFT (qui demande d'être administrateur).
//! - En mémoire, compact : chaque élément connaît son parent ; noms et clés (sans accents ni majuscules)
//!   sont bout à bout dans deux grands textes. Le chemin complet n'est refait que pour les résultats.
//! - Recherche : on trouve le mot cherché dans le grand texte des clés (memchr), puis on note seulement
//!   ces candidats. Des millions de noms restent instantanés.
//! - Sauvegardé sur le disque : au lancement suivant, tout est là tout de suite.

use crate::noms::{normaliser, score_avec};
use anyhow::{Result, bail};
use std::collections::BinaryHeap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

/// Pas de parent : une racine de disque.
const RACINE: u32 = u32::MAX;
const MAGIQUE: &[u8; 8] = b"FURETNM1";

/// Dossiers où personne ne cherche un fichier, mais qui en contiennent des millions : système, caches,
/// dépendances de développement, fichiers de jeux. Le dossier lui-même reste trouvable par son nom ; on n'y
/// entre pas. (Mesuré : 8 millions de noms et 535 Mo de mémoire sans cette liste.)
const NE_PAS_PARCOURIR: [&str; 12] = [
    "$recycle.bin", "system volume information", "winsxs", "$windows.~bt", "$windows.~ws", "config.msi", "appdata",
    "node_modules", "__pycache__", "steamapps", "$sysreset", "$getcurrent",
];
/// Les mêmes, mais seulement à la racine d'un disque (un projet peut avoir un dossier « windows »).
const NE_PAS_PARCOURIR_A_LA_RACINE: [&str; 4] = ["windows", "programdata", "recovery", "perflogs"];

/// Faut-il entrer dans ce dossier ? Non pour la liste ci-dessus et les dossiers cachés (« .git », « .cache »…).
fn a_parcourir(nom: &str, a_la_racine: bool) -> bool {
    let n = nom.to_lowercase();
    !(n.starts_with('.') || NE_PAS_PARCOURIR.contains(&n.as_str()) || (a_la_racine && NE_PAS_PARCOURIR_A_LA_RACINE.contains(&n.as_str())))
}
/// Résultats dans ces dossiers : en dessous des autres (fichiers du système, des programmes).
const DOSSIERS_DISCRETS: [&str; 8] = [
    "windows", "program files", "program files (x86)", "programdata", "appdata", "node_modules", "$recycle.bin", ".git",
];

#[derive(Clone, Copy)]
#[repr(C)]
struct Element {
    parent: u32,
    nom: u32,
    cle: u32,
    nom_long: u16,
    cle_long: u16,
    dossier: bool,
}

/// Tous les noms, compacts. Les clés sont séparées par '\n' pour que la recherche ne déborde pas d'un nom à l'autre.
pub struct ArbreNoms {
    elements: Vec<Element>,
    noms: String,
    cles: String,
}

/// Un nom trouvé.
pub struct Trouve {
    pub score: f32,
    pub chemin: PathBuf,
    pub dossier: bool,
}

impl ArbreNoms {
    pub fn vide() -> Self {
        ArbreNoms { elements: Vec::new(), noms: String::new(), cles: String::new() }
    }

    fn ajouter(&mut self, parent: u32, nom: &str, dossier: bool) -> u32 {
        let cle = normaliser(nom);
        let e = Element {
            parent,
            nom: self.noms.len() as u32,
            nom_long: nom.len().min(u16::MAX as usize) as u16,
            cle: self.cles.len() as u32,
            cle_long: cle.len().min(u16::MAX as usize) as u16,
            dossier,
        };
        self.noms.push_str(&nom[..e.nom_long as usize]);
        self.cles.push_str(&cle[..e.cle_long as usize]);
        self.cles.push('\n');
        self.elements.push(e);
        (self.elements.len() - 1) as u32
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Mémoire utilisée, en octets.
    pub fn memoire(&self) -> usize {
        self.elements.capacity() * size_of::<Element>() + self.noms.capacity() + self.cles.capacity()
    }

    fn nom(&self, e: &Element) -> &str {
        &self.noms[e.nom as usize..e.nom as usize + e.nom_long as usize]
    }

    fn cle(&self, e: &Element) -> &str {
        &self.cles[e.cle as usize..e.cle as usize + e.cle_long as usize]
    }

    pub fn chemin(&self, i: u32) -> PathBuf {
        let mut morceaux = Vec::new();
        let mut i = i;
        while i != RACINE {
            let e = &self.elements[i as usize];
            morceaux.push(self.nom(e));
            i = e.parent;
        }
        morceaux.reverse();
        // Les racines s'appellent « C: » : on remet la barre qui suit.
        let mut chemin = PathBuf::from(format!("{}\\", morceaux[0]));
        for m in &morceaux[1..] {
            chemin.push(m);
        }
        chemin
    }

    /// Discret si un dossier du chemin est un dossier de système ou de programmes.
    fn discret(&self, i: u32) -> bool {
        let mut i = self.elements[i as usize].parent;
        while i != RACINE {
            let e = &self.elements[i as usize];
            // Le vrai nom, pas la clé normalisée : « Program Files (x86) » y perd ses parenthèses, « .git » son point.
            let nom = self.nom(e).to_lowercase();
            if DOSSIERS_DISCRETS.contains(&nom.as_str()) || nom.starts_with('.') {
                return true;
            }
            i = e.parent;
        }
        false
    }

    /// L'élément dont la clé contient la position `p` du grand texte des clés.
    fn element_a(&self, p: usize) -> u32 {
        self.elements.partition_point(|e| e.cle as usize <= p) as u32 - 1
    }

    /// Les `n` meilleurs noms pour `recherche`, sans faute de frappe (la tolérance aux fautes, coûteuse,
    /// se fait sur les dossiers indexés seulement : voir `noms::IndexNoms`).
    pub fn chercher(&self, recherche: &str, n: usize) -> Vec<Trouve> {
        let q = normaliser(recherche);
        if q.is_empty() || n == 0 || self.elements.is_empty() {
            return Vec::new();
        }
        // Le grand texte des clés est partagé entre les cœurs ; chacun garde ses meilleurs candidats.
        let parts = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 8);
        let octets = self.cles.as_bytes();
        let taille = octets.len().div_ceil(parts);
        let plus_long = q.split(' ').max_by_key(|m| m.len()).unwrap_or(&q).to_owned();
        let tres_court = q.chars().count() < 3;
        let par_part: Vec<Vec<(Classement, u32)>> = std::thread::scope(|s| {
            let mut fils = Vec::new();
            for k in 0..parts {
                // Chaque part commence et finit sur une frontière de nom ('\n'), pour ne couper aucun nom.
                let debut = if k == 0 { 0 } else { memchr::memchr(b'\n', &octets[k * taille..]).map_or(octets.len(), |p| k * taille + p + 1) };
                let fin = if k + 1 == parts {
                    octets.len()
                } else {
                    memchr::memchr(b'\n', &octets[((k + 1) * taille).min(octets.len())..])
                        .map_or(octets.len(), |p| (k + 1) * taille + p + 1)
                };
                let (q, plus_long) = (&q, &plus_long);
                fils.push(s.spawn(move || {
                    let mut meilleurs: BinaryHeap<std::cmp::Reverse<(Classement, u32)>> = BinaryHeap::new();
                    if debut >= fin {
                        return Vec::new();
                    }
                    let tranche = &octets[debut..fin];
                    let mut garder = |i: u32| {
                        let e = &self.elements[i as usize];
                        let s = score_avec(q, self.cle(e), false);
                        if s > 0.0 {
                            // À score égal : les noms courts d'abord (le nom exact plutôt qu'un nom qui le contient).
                            meilleurs.push(std::cmp::Reverse((Classement(s - e.cle_long as f32 * 1e-4), i)));
                            if meilleurs.len() > n * 4 {
                                meilleurs.pop();
                            }
                        }
                    };
                    if tres_court {
                        // Une ou deux lettres : seulement les noms (ou leurs mots) qui commencent ainsi,
                        // repérés directement dans le texte : après un '\n' (début de nom) ou une espace (début de mot).
                        let premier = self.element_a(debut);
                        if self.cle(&self.elements[premier as usize]).starts_with(q.as_str()) {
                            garder(premier);
                        }
                        let mut dernier = premier;
                        for motif in [format!("\n{q}"), format!(" {q}")] {
                            for p in memchr::memmem::find_iter(tranche, motif.as_bytes()) {
                                let i = self.element_a(debut + p + 1);
                                if i != dernier {
                                    garder(i);
                                    dernier = i;
                                }
                            }
                        }
                    } else {
                        // Sinon : là où le mot le plus long (le plus sélectif) apparaît.
                        let mut dernier = u32::MAX;
                        for p in memchr::memmem::find_iter(tranche, plus_long.as_bytes()) {
                            let i = self.element_a(debut + p);
                            if i != dernier {
                                garder(i);
                                dernier = i;
                            }
                        }
                    }
                    meilleurs.into_iter().map(|std::cmp::Reverse(x)| x).collect()
                }));
            }
            fils.into_iter().map(|f| f.join().unwrap_or_default()).collect()
        });
        let meilleurs: Vec<(Classement, u32)> = par_part.into_iter().flatten().collect();
        // Les fichiers du système et des programmes passent derrière les autres.
        let mut trouves: Vec<Trouve> = meilleurs
            .into_iter()
            .map(|(Classement(s), i)| Trouve {
                score: if self.discret(i) { s - 0.2 } else { s },
                chemin: self.chemin(i),
                dossier: self.elements[i as usize].dossier,
            })
            .collect();
        trouves.sort_by(|a, b| b.score.total_cmp(&a.score));
        trouves.truncate(n);
        trouves
    }

    pub fn sauver(&self, fichier: &Path) -> Result<()> {
        let provisoire = fichier.with_extension("tmp");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&provisoire)?);
            f.write_all(MAGIQUE)?;
            for n in [self.elements.len(), self.noms.len(), self.cles.len()] {
                f.write_all(&(n as u64).to_le_bytes())?;
            }
            for e in &self.elements {
                f.write_all(&e.parent.to_le_bytes())?;
                f.write_all(&e.nom.to_le_bytes())?;
                f.write_all(&e.cle.to_le_bytes())?;
                f.write_all(&e.nom_long.to_le_bytes())?;
                f.write_all(&e.cle_long.to_le_bytes())?;
                f.write_all(&[e.dossier as u8])?;
            }
            f.write_all(self.noms.as_bytes())?;
            f.write_all(self.cles.as_bytes())?;
            f.flush()?;
        }
        std::fs::rename(&provisoire, fichier)?;
        Ok(())
    }

    pub fn charger(fichier: &Path) -> Result<Self> {
        let mut octets = Vec::new();
        std::fs::File::open(fichier)?.read_to_end(&mut octets)?;
        if octets.len() < 32 || &octets[..8] != MAGIQUE {
            bail!("fichier de noms inconnu");
        }
        let lire_u64 = |p: usize| u64::from_le_bytes(octets[p..p + 8].try_into().unwrap()) as usize;
        let (n, l_noms, l_cles) = (lire_u64(8), lire_u64(16), lire_u64(24));
        const TAILLE: usize = 4 + 4 + 4 + 2 + 2 + 1;
        let debut_noms = 32 + n * TAILLE;
        if octets.len() != debut_noms + l_noms + l_cles {
            bail!("fichier de noms abîmé");
        }
        let mut elements = Vec::with_capacity(n);
        for k in 0..n {
            let b = &octets[32 + k * TAILLE..32 + (k + 1) * TAILLE];
            elements.push(Element {
                parent: u32::from_le_bytes(b[0..4].try_into().unwrap()),
                nom: u32::from_le_bytes(b[4..8].try_into().unwrap()),
                cle: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                nom_long: u16::from_le_bytes(b[12..14].try_into().unwrap()),
                cle_long: u16::from_le_bytes(b[14..16].try_into().unwrap()),
                dossier: b[16] != 0,
            });
        }
        let noms = String::from_utf8(octets[debut_noms..debut_noms + l_noms].to_vec())?;
        let cles = String::from_utf8(octets[debut_noms + l_noms..].to_vec())?;
        Ok(ArbreNoms { elements, noms, cles })
    }
}

/// Un changement sur un disque, signalé par Windows.
pub enum Changement {
    Ajoute { chemin: PathBuf, dossier: bool },
    Retire(PathBuf),
    /// Trop de changements d'un coup : Windows n'a pas pu tout dire. Il faut refaire la liste.
    Debordement,
}

/// L'index des noms tel qu'on le cherche : la grande liste, et les changements arrivés depuis qu'elle a
/// été faite (ajouts cherchables, suppressions masquées). La grande liste est refaite de temps en temps.
pub struct IndexDisque {
    arbre: std::sync::RwLock<Arc<ArbreNoms>>,
    vivant: Mutex<Vivant>,
}

#[derive(Default)]
struct Vivant {
    /// (clé, chemin, dossier, moment)
    ajouts: Vec<(String, PathBuf, bool, std::time::Instant)>,
    /// Chemins retirés, en minuscules, avec le moment.
    retraits: std::collections::HashMap<String, std::time::Instant>,
}

/// Au-delà, il vaut mieux refaire la grande liste (les changements récents se cherchent un par un).
pub const CHANGEMENTS_MAX: usize = 50_000;

impl IndexDisque {
    pub fn nouveau(arbre: ArbreNoms) -> Self {
        IndexDisque { arbre: std::sync::RwLock::new(Arc::new(arbre)), vivant: Mutex::new(Vivant::default()) }
    }

    pub fn len(&self) -> usize {
        self.arbre.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Nombre de changements en attente de la prochaine grande liste.
    pub fn changements(&self) -> usize {
        let v = self.vivant.lock().unwrap_or_else(|e| e.into_inner());
        v.ajouts.len() + v.retraits.len()
    }

    pub fn noter(&self, c: Changement) {
        let maintenant = std::time::Instant::now();
        let mut v = self.vivant.lock().unwrap_or_else(|e| e.into_inner());
        match c {
            Changement::Ajoute { chemin, dossier } => {
                let bas = chemin.to_string_lossy().to_lowercase();
                v.retraits.remove(&bas);
                let nom = chemin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                v.ajouts.push((normaliser(&nom), chemin, dossier, maintenant));
            }
            Changement::Retire(chemin) => {
                let bas = chemin.to_string_lossy().to_lowercase();
                v.ajouts.retain(|(_, c, _, _)| c.to_string_lossy().to_lowercase() != bas);
                v.retraits.insert(bas, maintenant);
            }
            Changement::Debordement => {}
        }
    }

    /// Remplace la grande liste par une neuve, faite à partir de `depuis` : les changements arrivés
    /// avant y sont déjà ; ceux d'après restent notés à part.
    pub fn remplacer(&self, arbre: ArbreNoms, depuis: std::time::Instant) {
        *self.arbre.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(arbre);
        let mut v = self.vivant.lock().unwrap_or_else(|e| e.into_inner());
        v.ajouts.retain(|a| a.3 >= depuis);
        v.retraits.retain(|_, t| *t >= depuis);
    }

    pub fn sauver(&self, fichier: &Path) -> Result<()> {
        let arbre = self.arbre.read().unwrap_or_else(|e| e.into_inner()).clone();
        arbre.sauver(fichier)
    }

    pub fn memoire(&self) -> usize {
        self.arbre.read().unwrap_or_else(|e| e.into_inner()).memoire()
    }

    /// Les `n` meilleurs : la grande liste sans ce qui a été retiré depuis, plus les ajouts récents.
    pub fn chercher(&self, recherche: &str, n: usize) -> Vec<Trouve> {
        let arbre = self.arbre.read().unwrap_or_else(|e| e.into_inner()).clone();
        let (ajouts, retraits): (Vec<Trouve>, Vec<String>) = {
            let v = self.vivant.lock().unwrap_or_else(|e| e.into_inner());
            let q = normaliser(recherche);
            let ajouts = v
                .ajouts
                .iter()
                .filter_map(|(cle, chemin, dossier, _)| {
                    let s = score_avec(&q, cle, false);
                    (s > 0.0).then(|| Trouve { score: s, chemin: chemin.clone(), dossier: *dossier })
                })
                .collect();
            (ajouts, v.retraits.keys().cloned().collect())
        };
        // Un peu plus que n : certains peuvent avoir été retirés depuis.
        let mut trouves = arbre.chercher(recherche, n + retraits.len().min(20));
        if !retraits.is_empty() {
            trouves.retain(|t| {
                let bas = t.chemin.to_string_lossy().to_lowercase();
                // Retiré lui-même, ou dans un dossier retiré.
                !retraits.iter().any(|r| bas == *r || (bas.starts_with(r.as_str()) && bas.as_bytes().get(r.len()) == Some(&b'\\')))
            });
        }
        trouves.extend(ajouts);
        trouves.sort_by(|a, b| b.score.total_cmp(&a.score));
        trouves.dedup_by(|a, b| a.chemin == b.chemin);
        trouves.truncate(n);
        trouves
    }
}

/// Faut-il suivre un changement à ce chemin ? Pas dans les dossiers qu'on ne parcourt pas (AppData,
/// où les navigateurs écrivent sans arrêt, caches, système…).
fn a_suivre(relatif: &Path) -> bool {
    let composants: Vec<String> = relatif.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    // Tous les dossiers du chemin (pas le dernier élément, qui est le fichier ou dossier changé).
    composants.len() <= 1 || composants[..composants.len() - 1].iter().enumerate().all(|(k, c)| a_parcourir(c, k == 0))
}

/// Suit un disque en direct (sur le fil qui appelle, sans fin) : chaque fichier ou dossier créé, renommé
/// ou supprimé est signalé à `rappel`. Sans droits administrateur.
pub fn surveiller(racine: &Path, rappel: impl Fn(Changement)) -> Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
        FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        ReadDirectoryChangesW,
    };
    // SAFETY : la poignée est fermée à la sortie ; le tampon, aligné sur 4 octets, est lu dans ses limites
    // selon les décalages que Windows y écrit.
    unsafe {
        let h = CreateFileW(
            &windows::core::HSTRING::from(racine.as_os_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )?;
        let mut tampon = vec![0u32; 16 * 1024];
        loop {
            let mut lus = 0u32;
            let r = ReadDirectoryChangesW(
                h,
                tampon.as_mut_ptr().cast(),
                (tampon.len() * 4) as u32,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                Some(&mut lus),
                None,
                None,
            );
            if r.is_err() {
                let _ = CloseHandle(h);
                bail!("le suivi du disque s'est arrêté");
            }
            if lus == 0 {
                rappel(Changement::Debordement);
                continue;
            }
            let base = tampon.as_ptr() as *const u8;
            let mut decalage = 0usize;
            loop {
                let info = &*(base.add(decalage) as *const FILE_NOTIFY_INFORMATION);
                let nom = std::slice::from_raw_parts(info.FileName.as_ptr(), info.FileNameLength as usize / 2);
                let relatif = PathBuf::from(OsString::from_wide(nom));
                if a_suivre(&relatif) {
                    let chemin = racine.join(&relatif);
                    match info.Action {
                        a if a == FILE_ACTION_ADDED || a == FILE_ACTION_RENAMED_NEW_NAME => {
                            let dossier = chemin.is_dir();
                            rappel(Changement::Ajoute { chemin, dossier });
                        }
                        a if a == FILE_ACTION_REMOVED || a == FILE_ACTION_RENAMED_OLD_NAME => rappel(Changement::Retire(chemin)),
                        _ => {}
                    }
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                decalage += info.NextEntryOffset as usize;
            }
        }
    }
}

/// Ordre total sur les scores (f32), pour le tas.
#[derive(Clone, Copy, PartialEq)]
struct Classement(f32);
impl Eq for Classement {}
impl PartialOrd for Classement {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Classement {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// Les disques fixes (C:\, D:\…) : pas les clés USB, le réseau ni les lecteurs optiques.
pub fn disques_fixes() -> Vec<PathBuf> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_FIXED: u32 = 3;
    // SAFETY : simples requêtes sur des lettres de lecteur.
    let masque = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| masque & (1 << i) != 0)
        .map(|i| format!("{}:\\", char::from(b'A' + i)))
        .filter(|r| unsafe { GetDriveTypeW(&windows::core::HSTRING::from(r.as_str())) } == DRIVE_FIXED)
        .map(PathBuf::from)
        .collect()
}

/// Le contenu d'un dossier : (nom, est un dossier, est un lien à ne pas suivre).
fn lire_dossier(dossier: &Path) -> Vec<(OsString, bool, bool)> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
        FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
    };
    let mut motif: Vec<u16> = dossier.join("*").as_os_str().encode_wide().collect();
    motif.push(0);
    let mut donnees = WIN32_FIND_DATAW::default();
    let mut contenu = Vec::new();
    // SAFETY : `motif` est terminé par un zéro ; la poignée de recherche est fermée à la fin.
    unsafe {
        let Ok(h) = FindFirstFileExW(
            windows::core::PCWSTR(motif.as_ptr()),
            FindExInfoBasic,
            &mut donnees as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        ) else {
            return contenu;
        };
        loop {
            let long = donnees.cFileName.iter().position(|&c| c == 0).unwrap_or(donnees.cFileName.len());
            let nom = &donnees.cFileName[..long];
            if nom != [b'.' as u16] && nom != [b'.' as u16, b'.' as u16] {
                let attr = donnees.dwFileAttributes;
                contenu.push((
                    OsString::from_wide(nom),
                    attr & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                    attr & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0,
                ));
            }
            if FindNextFileW(h, &mut donnees).is_err() {
                break;
            }
        }
        let _ = FindClose(h);
    }
    contenu
}

/// Parcourt `racines` sur plusieurs fils et renvoie tous les noms.
pub fn parcourir(racines: &[PathBuf]) -> ArbreNoms {
    // Les dossiers à lire : (numéro dans l'arbre, chemin). Le fil principal seul écrit l'arbre.
    let file: Arc<(Mutex<(Vec<(u32, PathBuf)>, usize, bool)>, Condvar)> = Arc::new((Mutex::new((Vec::new(), 0, false)), Condvar::new()));
    let (envoi, reception) = mpsc::channel::<(u32, PathBuf, Vec<(OsString, bool, bool)>)>();
    let mut arbre = ArbreNoms::vide();
    {
        let (verrou, cond) = &*file;
        let mut f = verrou.lock().unwrap();
        for r in racines {
            let nom = r.to_string_lossy().trim_end_matches('\\').to_string();
            let i = arbre.ajouter(RACINE, &nom, true);
            f.0.push((i, r.clone()));
            f.1 += 1;
        }
        cond.notify_all();
    }
    let fils = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let mut poignees = Vec::new();
    for _ in 0..fils {
        let (file, envoi) = (file.clone(), envoi.clone());
        poignees.push(std::thread::spawn(move || {
            crate::processus::fil_en_priorite_basse();
            let (verrou, cond) = &*file;
            loop {
                let travail = {
                    let mut f = verrou.lock().unwrap();
                    loop {
                        if let Some(t) = f.0.pop() {
                            break Some(t);
                        }
                        if f.2 {
                            break None;
                        }
                        f = cond.wait(f).unwrap();
                    }
                };
                let Some((i, chemin)) = travail else { break };
                let contenu = lire_dossier(&chemin);
                if envoi.send((i, chemin, contenu)).is_err() {
                    break;
                }
            }
        }));
    }
    drop(envoi);
    let (verrou, cond) = &*file;
    let nb_racines = racines.len() as u32;
    for (parent, chemin, contenu) in reception.iter() {
        let mut nouveaux = Vec::new();
        for (nom, dossier, lien) in contenu {
            let nom = nom.to_string_lossy();
            let i = arbre.ajouter(parent, &nom, dossier);
            if dossier && !lien && a_parcourir(&nom, parent < nb_racines) {
                nouveaux.push((i, chemin.join(nom.as_ref())));
            }
        }
        let mut f = verrou.lock().unwrap();
        f.1 += nouveaux.len();
        f.1 -= 1;
        f.0.extend(nouveaux);
        if f.1 == 0 {
            f.2 = true;
        }
        cond.notify_all();
        if f.2 {
            break;
        }
    }
    for p in poignees {
        let _ = p.join();
    }
    arbre.elements.shrink_to_fit();
    arbre.noms.shrink_to_fit();
    arbre.cles.shrink_to_fit();
    arbre
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Tous les disques fixes : combien de noms, en combien de temps, quelle mémoire, quelle vitesse de recherche.
    /// cargo test --release -p furet-moteur disques -- --ignored --nocapture
    #[test]
    #[ignore]
    fn disques() {
        let racines = disques_fixes();
        let t = Instant::now();
        let arbre = parcourir(&racines);
        println!(
            "{:?} : {} noms en {:.1} s, {} Mo en mémoire",
            racines,
            arbre.len(),
            t.elapsed().as_secs_f64(),
            arbre.memoire() >> 20
        );
        let fichier = std::env::temp_dir().join("furet-noms-essai.bin");
        let t = Instant::now();
        arbre.sauver(&fichier).unwrap();
        let taille = std::fs::metadata(&fichier).unwrap().len() >> 20;
        let ecrit = t.elapsed().as_millis();
        let t = Instant::now();
        let relu = ArbreNoms::charger(&fichier).unwrap();
        println!("sauvegarde {taille} Mo en {ecrit} ms, rechargée en {} ms", t.elapsed().as_millis());
        let _ = std::fs::remove_file(&fichier);
        for q in ["minecraft", "facture", "a", "re", "readme", "photo vacances", "suspicious"] {
            let t = Instant::now();
            let r = relu.chercher(q, 8);
            println!("« {q} » : {} résultat(s) en {:.1} ms", r.len(), t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
