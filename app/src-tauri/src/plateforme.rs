//! Ce que l'indexation doit savoir du système pour se mettre en pause : le PC est-il sur batterie, un jeu
//! (ou une appli en plein écran) est-il au premier plan ? Windows, et Linux pour la batterie.

/// Le PC fonctionne sur batterie (secteur débranché).
pub fn sur_batterie() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut etat = SYSTEM_POWER_STATUS::default();
        // SAFETY : simple lecture dans une structure locale.
        // ACLineStatus : 0 = sur batterie, 1 = sur secteur, 255 = inconnu (PC fixe : jamais en pause).
        unsafe { GetSystemPowerStatus(&mut etat).is_ok() && etat.ACLineStatus == 0 }
    }
    #[cfg(target_os = "linux")]
    {
        // Une alimentation secteur (« Mains ») débranchée, alors qu'il y a une batterie.
        let Ok(alims) = std::fs::read_dir("/sys/class/power_supply") else { return false };
        let mut secteur_branche = None;
        let mut batterie = false;
        for a in alims.flatten() {
            let lire = |f: &str| std::fs::read_to_string(a.path().join(f)).unwrap_or_default().trim().to_owned();
            match lire("type").as_str() {
                "Mains" => secteur_branche = Some(secteur_branche.unwrap_or(false) || lire("online") == "1"),
                "Battery" => batterie = true,
                _ => {}
            }
        }
        batterie && secteur_branche == Some(false)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Un jeu, une vidéo ou une présentation occupe tout l'écran de l'île (repris de Quiera,
/// platform/windows/fullscreen.rs). Ailleurs que sous Windows : pas encore détecté.
pub fn jeu_en_cours() -> bool {
    #[cfg(windows)]
    {
        windows_plein_ecran::premier_plan_en_plein_ecran()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod windows_plein_ecran {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Shell::{QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowLongPtrW,
        GetWindowRect, GetWindowThreadProcessId, IsZoomed, WS_EX_TOOLWINDOW,
    };

    /// Fenêtres de l'interpréteur de commandes qui couvrent l'écran sans être une appli.
    const CLASSES_SHELL: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

    pub fn premier_plan_en_plein_ecran() -> bool {
        // SAFETY : simples requêtes Win32 sur des poignées de fenêtres ; une poignée périmée les fait
        // échouer, ce qui se lit « pas en plein écran ».
        unsafe {
            // Direct3D exclusif ou mode présentation : Windows le dit lui-même.
            if SHQueryUserNotificationState().is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE) {
                return true;
            }
            let fg = GetForegroundWindow();
            if fg.is_invalid() || ignoree(fg) || IsZoomed(fg).as_bool() {
                return false;
            }
            // Une fenêtre sans bord qui couvre tout l'écran (le plein écran des jeux récents).
            let ecran = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
            if ecran.is_invalid() {
                return false;
            }
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let mut rect = RECT::default();
            GetMonitorInfoW(ecran, &mut info).as_bool() && GetWindowRect(fg, &mut rect).is_ok() && couvre(rect, info.rcMonitor)
        }
    }

    fn couvre(f: RECT, e: RECT) -> bool {
        f.left <= e.left && f.top <= e.top && f.right >= e.right && f.bottom >= e.bottom
    }

    /// Le bureau, la barre des tâches, une fenêtre outil (Alt+Tab, menus), ou Furet lui-même.
    unsafe fn ignoree(fg: HWND) -> bool {
        unsafe {
            if fg == GetShellWindow() || fg == GetDesktopWindow() {
                return true;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid == GetCurrentProcessId() || GetWindowLongPtrW(fg, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return true;
            }
            let mut classe = [0u16; 64];
            let n = GetClassNameW(fg, &mut classe) as usize;
            CLASSES_SHELL.contains(&String::from_utf16_lossy(&classe[..n]).as_str())
        }
    }
}
