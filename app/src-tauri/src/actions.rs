//! Les actions sur un résultat : copier son chemin, l'ouvrir avec une autre appli, le glisser vers une
//! autre appli ou un dossier. Le glisser et « Ouvrir avec » sont pour l'instant propres à Windows.

use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{historique, raccourci, recherche};

fn chemin(cle: &str) -> Result<PathBuf, String> {
    recherche::chemin_offert(cle).ok_or_else(|| "fichier inconnu : cherchez à nouveau".into())
}

#[tauri::command]
pub fn copier_chemin(app: AppHandle, cle: String) -> Result<(), String> {
    let c = chemin(&cle)?;
    historique::noter(&c);
    app.clipboard().write_text(c.to_string_lossy().into_owned()).map_err(|e| e.to_string())
}

/// La fenêtre « Ouvrir avec » du système ; la barre se ferme.
#[tauri::command]
pub fn ouvrir_avec(app: AppHandle, cle: String) -> Result<(), String> {
    let c = chemin(&cle)?;
    historique::noter(&c);
    raccourci::fermer(&app, false);
    #[cfg(windows)]
    std::thread::spawn(move || systeme::ouvrir_avec(&c));
    #[cfg(not(windows))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener().open_path(c.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Commence à glisser le fichier (le bouton de la souris est encore enfoncé) : il est copié là où on le lâche.
#[tauri::command]
pub fn glisser(app: AppHandle, cle: String) -> Result<(), String> {
    let c = chemin(&cle)?;
    historique::noter(&c);
    #[cfg(windows)]
    {
        app.run_on_main_thread(move || {
            if let Err(e) = systeme::glisser(&c) {
                log::warn!("glisser : {e}");
            }
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, c);
        Err("pas encore possible sur ce système".into())
    }
}

#[cfg(windows)]
mod systeme {
    use std::path::Path;

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IDataObject};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, IDropSource};
    use windows::Win32::UI::Shell::{
        BHID_DataObject, IShellItem, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPENASINFO,
        SHCreateItemFromParsingName, SHDoDragDrop, SHOpenWithDialog,
    };
    use windows::core::{HSTRING, PCWSTR};

    pub fn ouvrir_avec(chemin: &Path) {
        let nom = HSTRING::from(chemin.as_os_str());
        let info = OPENASINFO {
            pcszFile: PCWSTR(nom.as_ptr()),
            pcszClass: PCWSTR::null(),
            oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
        };
        // SAFETY : `nom` vit jusqu'à la fin de l'appel ; COM est initialisé pour ce fil.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let _ = SHOpenWithDialog(None, &info);
        }
    }

    /// Sur le fil principal (OLE y est initialisé) ; l'interpréteur dessine l'image du fichier sous la souris.
    pub fn glisser(chemin: &Path) -> windows::core::Result<()> {
        // SAFETY : objets COM créés et libérés ici ; l'appel attend que le fichier soit lâché.
        unsafe {
            let element: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(chemin.as_os_str()), None)?;
            let donnees: IDataObject = element.BindToHandler(None, &BHID_DataObject)?;
            // Copier ou faire un raccourci, jamais déplacer : le fichier de l'utilisateur reste où il est.
            SHDoDragDrop(crate::ile::hwnd(), &donnees, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_LINK)?;
        }
        Ok(())
    }
}
