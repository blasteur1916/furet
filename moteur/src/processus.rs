//! Lancement des programmes du moteur (llama-server, ffmpeg, sherpa-onnx…).

use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::process::Command;

/// Sans fenêtre de console : l'appli n'en a pas, et chaque programme lancé en ouvrirait une sinon.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Priorité basse : l'indexation passe après tout ce que fait l'utilisateur.
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

pub fn commande(programme: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(programme);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

static RACINE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Le dossier de Furet (outils, modèles), pour trouver ses programmes.
pub fn definir_racine(racine: &std::path::Path) {
    let _ = RACINE.set(racine.to_path_buf());
}

/// ffmpeg ou ffprobe : celui installé avec Furet (`outils/ffmpeg`), sinon celui du système.
pub fn ffmpeg(nom: &str) -> std::path::PathBuf {
    let exe = if cfg!(windows) { format!("{nom}.exe") } else { nom.to_owned() };
    RACINE
        .get()
        .map(|r| r.join("outils").join("ffmpeg").join(&exe))
        .filter(|p| p.exists())
        .unwrap_or_else(|| exe.into())
}

/// Pour le travail d'indexation (ffmpeg, llama-server d'indexation, transcription) : sans fenêtre et
/// en priorité basse, pour que le PC — et la barre de recherche — restent fluides.
pub fn commande_basse(programme: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(programme);
    c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    c
}

/// Tout le programme en priorité basse (le prototype en ligne de commande, qui ne fait qu'indexer).
pub fn programme_en_priorite_basse() {
    use windows::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
    // SAFETY : la pseudo-poignée du processus courant est toujours valide.
    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

/// Le fil qui appelle passe en priorité basse (fils de préparation de l'indexation).
pub fn fil_en_priorite_basse() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    // SAFETY : la pseudo-poignée du fil courant est toujours valide.
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Lie les programmes lancés à celui-ci : quand il s'arrête, même brutalement (plantage, Ctrl+C,
/// fin de tâche), Windows arrête aussi llama-server, ffmpeg et la transcription. Sans cela, un
/// llama-server de 1,5 Go pourrait rester en mémoire. À appeler une fois, au démarrage.
pub fn lier_au_programme() {
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    // SAFETY : la poignée du job reste ouverte jusqu'à la fin du programme (c'est voulu : sa
    // fermeture, par Windows à la sortie, est ce qui arrête les programmes enfants).
    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else { return };
        let mut limites = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limites.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limites as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let _ = AssignProcessToJobObject(job, GetCurrentProcess());
    }
}
