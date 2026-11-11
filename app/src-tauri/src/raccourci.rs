//! Le raccourci ouvre la barre (repris du lanceur de Quiera) : l'île prend le clavier pendant qu'elle est
//! ouverte, puis le rend à la fenêtre qui l'avait. Alt+Espace, ou le premier raccourci libre.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, IsWindow, MSG, PM_NOREMOVE, PeekMessageW, SetForegroundWindow, WM_HOTKEY,
};

use crate::{ile, recherche};

pub const EVT_OUVRIR: &str = "furet://ouvrir";
pub const EVT_FERMER: &str = "furet://fermer";
/// Le temps laissé à Windows pour donner le clavier à l'île.
const VERIFICATION_CLAVIER: Duration = Duration::from_millis(400);

/// Dans l'ordre de préférence ; le premier qu'aucun autre programme ne tient est gardé.
const RACCOURCIS: [(&str, u32); 3] = [
    ("Alt+Espace", MOD_ALT.0),
    ("Ctrl+Alt+Espace", MOD_CONTROL.0 | MOD_ALT.0),
    ("Ctrl+Maj+Espace", MOD_CONTROL.0 | MOD_SHIFT.0),
];

static OUVERT: AtomicBool = AtomicBool::new(false);
/// Le raccourci obtenu (« Alt+Espace »…).
static TENU: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Le raccourci qui ouvre la barre, pour le montrer.
#[tauri::command]
pub fn raccourci_actuel() -> String {
    TENU.get().map_or_else(|| "aucun (tous pris par d'autres programmes)".into(), |r| (*r).into())
}

/// Au premier lancement : l'île s'ouvre d'elle-même sur l'accueil.
pub fn ouvrir_au_lancement(app: &AppHandle) {
    ouvrir(app, None, false);
}

/// Ce que reçoit l'interface à l'ouverture. Miroir de `Ouverture` dans `src/ipc/ile.ts`.
#[derive(Clone, serde::Serialize)]
struct Ouverture {
    /// Texte de départ (mode de test : `furet.exe --ouvrir "texte"`).
    texte: Option<String>,
    /// Mode de test (`FURET_TEST=1`) : la barre ne se ferme pas quand elle perd le clavier, pour
    /// pouvoir la regarder sans taper dans les autres fenêtres.
    test: bool,
    /// Ouvrir directement les réglages (mode de test : `furet.exe --reglages`).
    reglages: bool,
    /// Ouvrir l'aperçu de ce résultat dès qu'il est là (mode de test : `--ouvrir "texte" --apercu 2`).
    apercu: Option<usize>,
    /// Les outils et le modèle ne sont pas encore là : l'accueil s'ouvre à la place de la barre.
    accueil: bool,
}

fn mode_test() -> bool {
    std::env::var_os("FURET_TEST").is_some()
}

/// Un deuxième lancement avec `--ouvrir ["texte"]` ou `--fermer`, en mode de test seulement.
pub fn ligne_de_commande(app: &AppHandle, args: &[String]) {
    if !mode_test() {
        return;
    }
    match args.iter().position(|a| a == "--ouvrir" || a == "--reglages" || a == "--fermer") {
        Some(i) if args[i] == "--ouvrir" => {
            let apercu = args.iter().position(|a| a == "--apercu").map(|k| args.get(k + 1).and_then(|n| n.parse().ok()).unwrap_or(0));
            fermer(app, false);
            ouvrir_avec(app, args.get(i + 1).cloned(), false, apercu);
        }
        Some(i) if args[i] == "--reglages" => {
            fermer(app, false);
            ouvrir(app, None, true);
        }
        Some(_) => fermer(app, false),
        None => {}
    }
}
/// La fenêtre qui avait le clavier avant l'île.
static PRECEDENTE: AtomicIsize = AtomicIsize::new(0);

/// Lance le fil qui tient le raccourci. Les appuis arrivent dans sa file de messages.
pub fn demarrer(app: &AppHandle) {
    let app = app.clone();
    let lance = std::thread::Builder::new().name("furet-raccourci".into()).spawn(move || {
        // SAFETY : raccourci de fil (sans fenêtre) : WM_HOTKEY arrive dans la file de ce fil, lue ci-dessous.
        unsafe {
            let mut msg = MSG::default();
            // Crée la file de messages avant d'enregistrer le raccourci.
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let tenu = RACCOURCIS.iter().find(|(_, modif)| {
                RegisterHotKey(None, 1, HOT_KEY_MODIFIERS(modif | MOD_NOREPEAT.0), u32::from(VK_SPACE.0)).is_ok()
            });
            match tenu {
                Some((nom, _)) => {
                    log::info!("raccourci : {nom}");
                    let _ = TENU.set(nom);
                }
                None => {
                    log::error!("aucun raccourci libre : un autre programme les tient tous");
                    return;
                }
            }
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY {
                    if OUVERT.load(Ordering::Relaxed) {
                        fermer(&app, true);
                    } else {
                        ouvrir(&app, None, false);
                    }
                }
            }
        }
    });
    if let Err(e) = lance {
        log::error!("le raccourci n'a pas pu démarrer : {e}");
    }
}

/// Appelé sur le fil du raccourci : Windows laisse le programme qui vient de recevoir la touche
/// prendre le premier plan, donc tout se fait tout de suite.
fn ouvrir(app: &AppHandle, texte: Option<String>, reglages: bool) {
    ouvrir_avec(app, texte, reglages, None);
}

fn ouvrir_avec(app: &AppHandle, texte: Option<String>, reglages: bool, apercu: Option<usize>) {
    if OUVERT.swap(true, Ordering::Relaxed) {
        return;
    }
    let test = mode_test();
    ile::remettre_au_dessus();
    // SAFETY : lecture du premier plan, et demande pour notre propre fenêtre.
    unsafe {
        PRECEDENTE.store(GetForegroundWindow().0 as isize, Ordering::Relaxed);
        if let Some(h) = ile::hwnd() {
            let _ = SetForegroundWindow(h);
        }
    }
    if let Some(fenetre) = app.get_webview_window(ile::LABEL) {
        let _ = fenetre.set_focus();
    }
    let accueil = !crate::installation::essentiel_pret();
    let _ = app.emit(EVT_OUVRIR, Ouverture { texte, test, reglages, apercu, accueil });
    recherche::rafraichir_si_change();
    if test {
        return;
    }
    // Windows peut refuser le clavier (un autre programme tient le premier plan) : une barre où l'on ne
    // peut pas taper ne ferait que gêner, elle se referme.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(VERIFICATION_CLAVIER);
        // SAFETY : simple lecture.
        let premier_plan = unsafe { GetForegroundWindow() };
        if OUVERT.load(Ordering::Relaxed) && ile::hwnd() != Some(premier_plan) {
            log::warn!("Windows n'a pas donné le clavier à l'île : elle se referme");
            fermer(&app, false);
        }
    });
}

/// Ferme la barre ; `rendre` redonne le clavier à la fenêtre d'avant (pas quand on vient d'ouvrir un fichier).
pub fn fermer(app: &AppHandle, rendre: bool) {
    if !OUVERT.swap(false, Ordering::Relaxed) {
        return;
    }
    let precedente = PRECEDENTE.swap(0, Ordering::Relaxed);
    if rendre && precedente != 0 {
        let h = HWND(precedente as *mut c_void);
        // SAFETY : une poignée périmée est vérifiée d'abord ; l'appel échoue seulement dans ce cas.
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                let _ = SetForegroundWindow(h);
            }
        }
    }
    let _ = app.emit(EVT_FERMER, ());
}

#[tauri::command]
pub fn fermer_barre(app: AppHandle, rendre: bool) {
    fermer(&app, rendre);
}
