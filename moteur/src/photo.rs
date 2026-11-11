//! Décodage des images avec WIC (le décodeur de Windows), dans le bon sens.

use anyhow::{Result, bail};
use std::path::Path;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToUInt16};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::core::{GUID, HSTRING, Interface, w};

pub struct Images {
    wic: IWICImagingFactory,
}

/// Une image ouverte. `largeur` et `hauteur` sont celles de l'image remise droite.
pub struct Source {
    image: IWICBitmapFrameDecode,
    rotation: WICBitmapTransformOptions,
    pub largeur: u32,
    pub hauteur: u32,
}

/// Pixels décodés : 3 octets (BGR) ou 4 octets (BGRA) par point.
pub struct Pixels {
    pub largeur: u32,
    pub hauteur: u32,
    pub octets: Vec<u8>,
}

impl Images {
    pub fn nouveau() -> Result<Self> {
        unsafe {
            // Déjà initialisé sur ce fil : ce n'est pas une erreur.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            Ok(Images { wic: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? })
        }
    }

    pub fn ouvrir(&self, chemin: &Path) -> Result<Source> {
        unsafe {
            let decodeur = self.wic.CreateDecoderFromFilename(
                &HSTRING::from(chemin.as_os_str()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;
            let image = decodeur.GetFrame(0)?;
            let rotation = transformation_exif(orientation(&image));
            let (mut l, mut h) = (0u32, 0u32);
            image.GetSize(&mut l, &mut h)?;
            if l == 0 || h == 0 {
                bail!("image vide");
            }
            if tourne_d_un_quart(rotation) {
                std::mem::swap(&mut l, &mut h);
            }
            Ok(Source { image, rotation, largeur: l, hauteur: h })
        }
    }

    /// Réduit l'image pour que son plus grand côté fasse au plus `cote` (jamais agrandie).
    pub fn pixels(&self, s: &Source, cote: u32, bgra: bool) -> Result<Pixels> {
        let echelle = (cote as f64 / s.largeur.max(s.hauteur) as f64).min(1.0);
        let l = ((s.largeur as f64 * echelle).round() as u32).max(1);
        let h = ((s.hauteur as f64 * echelle).round() as u32).max(1);
        let (format, taille_point): (GUID, u32) =
            if bgra { (GUID_WICPixelFormat32bppBGRA, 4) } else { (GUID_WICPixelFormat24bppBGR, 3) };
        unsafe {
            // Réduire d'abord, tourner ensuite : tourner l'image entière ferait relire le JPEG à chaque ligne.
            let reducteur = self.wic.CreateBitmapScaler()?;
            let (lr, hr) = if tourne_d_un_quart(s.rotation) { (h, l) } else { (l, h) };
            reducteur.Initialize(&s.image, lr, hr, WICBitmapInterpolationModeFant)?;
            let droite: IWICBitmapSource = if s.rotation == WICBitmapTransformRotate0 {
                reducteur.cast()?
            } else {
                let en_memoire = self.wic.CreateBitmapFromSource(&reducteur, WICBitmapCacheOnLoad)?;
                let rotateur = self.wic.CreateBitmapFlipRotator()?;
                rotateur.Initialize(&en_memoire, s.rotation)?;
                rotateur.cast()?
            };
            let convertisseur = self.wic.CreateFormatConverter()?;
            convertisseur.Initialize(&droite, &format, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            let pas = l * taille_point;
            let mut octets = vec![0u8; pas as usize * h as usize];
            convertisseur.CopyPixels(std::ptr::null(), pas, &mut octets)?;
            Ok(Pixels { largeur: l, hauteur: h, octets })
        }
    }

    /// JPEG réduit, prêt à envoyer au modèle.
    pub fn jpeg(&self, s: &Source, cote: u32) -> Result<Vec<u8>> {
        let p = self.pixels(s, cote, false)?;
        encoder_jpeg(&p)
    }
}

pub fn encoder_jpeg(p: &Pixels) -> Result<Vec<u8>> {
    let rgb: Vec<u8> = p.octets.chunks_exact(3).flat_map(|b| [b[2], b[1], b[0]]).collect();
    let mut jpeg = Vec::new();
    ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode(
        &rgb,
        p.largeur,
        p.hauteur,
        ::image::ExtendedColorType::Rgb8,
    )?;
    Ok(jpeg)
}

/// Orientation EXIF (1 à 8), 1 si absente. Fonctionne pour JPEG, HEIC, TIFF et RAW.
unsafe fn orientation(image: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(lecteur) = image.GetMetadataQueryReader() else { return 1 };
        let mut valeur = PROPVARIANT::default();
        if lecteur.GetMetadataByName(w!("System.Photo.Orientation"), &mut valeur).is_err() {
            return 1;
        }
        let o = PropVariantToUInt16(&valeur).unwrap_or(1);
        let _ = PropVariantClear(&mut valeur);
        o
    }
}

/// Rotate90 (1) et Rotate270 (3 = 90 + 180) ont ce bit ; Rotate180 (2) ne l'a pas.
fn tourne_d_un_quart(r: WICBitmapTransformOptions) -> bool {
    r.0 & WICBitmapTransformRotate90.0 != 0
}

/// Rotation WIC (sens horaire) qui remet l'image droite.
fn transformation_exif(orientation: u16) -> WICBitmapTransformOptions {
    match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0),
        8 => WICBitmapTransformRotate270,
        _ => WICBitmapTransformRotate0,
    }
}
