//! Mémoire vive (pics Windows) et mémoire de la carte graphique (nvidia-smi).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;

/// Pic de mémoire vive (working set) d'un processus, en octets.
pub fn pic_memoire(processus: HANDLE) -> u64 {
    let mut c = PROCESS_MEMORY_COUNTERS::default();
    let taille = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    match unsafe { GetProcessMemoryInfo(processus, &mut c, taille) } {
        Ok(()) => c.PeakWorkingSetSize as u64,
        Err(_) => 0,
    }
}

pub fn pic_memoire_programme() -> u64 {
    pic_memoire(unsafe { GetCurrentProcess() })
}

/// Mémoire utilisée sur la première carte NVIDIA, en octets (None sans nvidia-smi).
fn vram_utilisee() -> Option<u64> {
    let sortie = crate::processus::commande("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits", "-i", "0"])
        .output()
        .ok()?;
    let mio: u64 = String::from_utf8_lossy(&sortie.stdout).trim().parse().ok()?;
    Some(mio << 20)
}

/// Relève la mémoire de la carte graphique chaque seconde et garde le pic.
pub struct SuiviVram {
    pub depart: Option<u64>,
    pic: Arc<AtomicU64>,
    arret: Arc<AtomicBool>,
    fil: Option<JoinHandle<()>>,
}

impl SuiviVram {
    pub fn demarrer() -> Self {
        let depart = vram_utilisee();
        let pic = Arc::new(AtomicU64::new(depart.unwrap_or(0)));
        let arret = Arc::new(AtomicBool::new(false));
        let fil = depart.map(|_| {
            let (pic, arret) = (pic.clone(), arret.clone());
            std::thread::spawn(move || {
                while !arret.load(Ordering::Relaxed) {
                    if let Some(v) = vram_utilisee() {
                        pic.fetch_max(v, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
        });
        SuiviVram { depart, pic, arret, fil }
    }

    /// Pic au-dessus de la mémoire déjà utilisée au départ, en octets.
    pub fn arreter(mut self) -> Option<u64> {
        self.arret.store(true, Ordering::Relaxed);
        if let Some(f) = self.fil.take() {
            let _ = f.join();
        }
        self.depart.map(|d| self.pic.load(Ordering::Relaxed).saturating_sub(d))
    }
}

/// Place prise par l'index (la base et ses fichiers de journal SQLite), en octets.
pub fn taille_base(chemin: &std::path::Path) -> u64 {
    ["", "-wal", "-shm"]
        .iter()
        .filter_map(|s| std::fs::metadata(format!("{}{s}", chemin.display())).ok())
        .map(|m| m.len())
        .sum()
}

pub fn mo(octets: u64) -> String {
    if octets < 10 << 20 {
        format!("{} Ko", octets >> 10)
    } else {
        format!("{} Mo", octets >> 20)
    }
}

pub fn secondes(ms: f64) -> String {
    format!("{:.2} s", ms / 1000.0).replace('.', ",")
}
