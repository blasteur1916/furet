//! OCR intégré à Windows (Windows.Media.Ocr) : gratuit, local, langues du profil.

use crate::photo::Pixels;
use anyhow::{Context, Result};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

pub struct Ocr {
    moteur: OcrEngine,
    pub cote_max: u32,
}

impl Ocr {
    pub fn nouveau() -> Result<Self> {
        let moteur = OcrEngine::TryCreateFromUserProfileLanguages().context("OCR Windows indisponible")?;
        Ok(Ocr { moteur, cote_max: OcrEngine::MaxImageDimension()? })
    }

    pub fn langue(&self) -> String {
        self.moteur.RecognizerLanguage().and_then(|l| l.DisplayName()).map(|n| n.to_string()).unwrap_or_default()
    }

    /// Lit le texte d'une image BGRA.
    pub fn lire(&self, p: &Pixels) -> Result<String> {
        let ecrivain = DataWriter::new()?;
        ecrivain.WriteBytes(&p.octets)?;
        let image = SoftwareBitmap::CreateCopyFromBuffer(
            &ecrivain.DetachBuffer()?,
            BitmapPixelFormat::Bgra8,
            p.largeur as i32,
            p.hauteur as i32,
        )?;
        let resultat = self.moteur.RecognizeAsync(&image)?.join()?;
        let mut texte = String::new();
        for ligne in resultat.Lines()? {
            texte.push_str(&ligne.Text()?.to_string());
            texte.push('\n');
        }
        Ok(texte)
    }
}

/// Assez de vrais mots pour valoir la peine d'être indexé (une photo de paysage donne souvent du bruit).
pub fn texte_utile(texte: &str) -> bool {
    let mots = texte.split_whitespace().filter(|m| m.chars().filter(|c| c.is_alphabetic()).count() >= 3).count();
    mots >= 4
}
