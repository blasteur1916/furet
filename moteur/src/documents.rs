//! Texte des documents : Word, PowerPoint, OpenDocument, tableurs, texte et code.

use anyhow::{Context, Result, bail};
use calamine::Reader as _;
use quick_xml::events::Event;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Au-delà, on n'indexe que le début du fichier (gros journaux, exports…).
const TEXTE_MAX_OCTETS: u64 = 2 << 20;
const XML_MAX_OCTETS: u64 = 64 << 20;
const LIGNES_CARACTERES: usize = 1000;

/// Un bloc de texte et sa position : diapo, feuille ou première ligne.
pub struct Bloc {
    pub nature: &'static str,
    pub numero: f64,
    pub texte: String,
}

pub fn office(chemin: &Path, ext: &str) -> Result<Vec<Bloc>> {
    let mut zip = zip::ZipArchive::new(File::open(chemin)?).context("fichier Office illisible")?;
    let lire = |zip: &mut zip::ZipArchive<File>, nom: &str| -> Result<String> {
        let mut s = String::new();
        zip.by_name(nom)?.take(XML_MAX_OCTETS).read_to_string(&mut s)?;
        Ok(s)
    };
    match ext {
        "docx" | "docm" => {
            let xml = lire(&mut zip, "word/document.xml")?;
            Ok(vec![Bloc { nature: "texte", numero: 0.0, texte: texte_xml(&xml, Some("t"), None)?.concat() }])
        }
        "pptx" | "pptm" => {
            let mut diapos: Vec<(u32, String)> = zip
                .file_names()
                .filter_map(|n| n.ok())
                .filter_map(|n| {
                    let num = n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
                    Some((num, n.to_string()))
                })
                .collect();
            diapos.sort();
            diapos
                .into_iter()
                .map(|(num, nom)| {
                    let xml = lire(&mut zip, &nom)?;
                    Ok(Bloc { nature: "diapo", numero: num as f64, texte: texte_xml(&xml, Some("t"), None)?.concat() })
                })
                .collect()
        }
        "odp" => {
            let xml = lire(&mut zip, "content.xml")?;
            Ok(texte_xml(&xml, None, Some("page"))?
                .into_iter()
                .enumerate()
                .map(|(i, texte)| Bloc { nature: "diapo", numero: (i + 1) as f64, texte })
                .collect())
        }
        _ => {
            let xml = lire(&mut zip, "content.xml")?;
            Ok(vec![Bloc { nature: "texte", numero: 0.0, texte: texte_xml(&xml, None, None)?.concat() }])
        }
    }
}

/// Texte d'un XML Office. `dans` : ne garder que le texte de cette balise (w:t, a:t).
/// `page` : commencer un nouveau bloc à chaque balise de ce nom (draw:page).
fn texte_xml(xml: &str, dans: Option<&str>, page: Option<&str>) -> Result<Vec<String>> {
    let mut lecteur = quick_xml::Reader::from_str(xml);
    let mut blocs = if page.is_some() { Vec::new() } else { vec![String::new()] };
    let mut profondeur = 0;
    loop {
        let garder = dans.is_none() || profondeur > 0;
        match lecteur.read_event()? {
            Event::Start(e) => {
                let nom = e.local_name();
                if Some(nom.as_ref()) == dans {
                    profondeur += 1;
                } else if Some(nom.as_ref()) == page {
                    blocs.push(String::new());
                }
            }
            Event::End(e) => {
                let nom = e.local_name();
                if Some(nom.as_ref()) == dans {
                    profondeur -= 1;
                } else if matches!(nom.as_ref(), "p" | "h") {
                    if let Some(b) = blocs.last_mut() {
                        b.push('\n');
                    }
                }
            }
            Event::Empty(e) => {
                let separateur = match e.local_name().as_ref() {
                    "tab" | "s" => Some(' '),
                    "br" | "line-break" => Some('\n'),
                    _ => None,
                };
                if let (Some(c), Some(b)) = (separateur, blocs.last_mut()) {
                    b.push(c);
                }
            }
            Event::Text(t) if garder => {
                if let Some(b) = blocs.last_mut() {
                    b.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) if garder => {
                let c = match r.resolve_char_ref()? {
                    Some(c) => Some(c),
                    None => match r.as_ref() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    },
                };
                if let (Some(c), Some(b)) = (c, blocs.last_mut()) {
                    b.push(c);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(blocs)
}

/// Excel, ODS : un bloc par feuille, cellules séparées par « | ».
pub fn tableur(chemin: &Path) -> Result<Vec<Bloc>> {
    let mut classeur = calamine::open_workbook_auto(chemin).context("tableur illisible")?;
    let mut blocs = Vec::new();
    for (i, nom) in classeur.sheet_names().iter().enumerate() {
        let plage = classeur.worksheet_range(nom)?;
        let mut texte = String::new();
        for ligne in plage.rows() {
            let cellules: Vec<String> = ligne.iter().map(|c| c.to_string()).filter(|s| !s.trim().is_empty()).collect();
            if !cellules.is_empty() {
                texte.push_str(&cellules.join(" | "));
                texte.push('\n');
            }
            if texte.len() as u64 > TEXTE_MAX_OCTETS {
                break;
            }
        }
        blocs.push(Bloc { nature: "feuille", numero: (i + 1) as f64, texte });
    }
    Ok(blocs)
}

/// Texte brut et code : des groupes de lignes, repérés par leur première ligne.
pub fn texte(chemin: &Path, html: bool) -> Result<Vec<Bloc>> {
    let mut octets = Vec::new();
    File::open(chemin)?.take(TEXTE_MAX_OCTETS).read_to_end(&mut octets)?;
    let mut texte = decoder(&octets)?;
    if html {
        texte = sans_balises(&texte);
    }
    let mut blocs = Vec::new();
    let mut courant = String::new();
    let mut debut = 1;
    for (i, ligne) in texte.lines().enumerate() {
        if !courant.is_empty() && courant.len() + ligne.len() > LIGNES_CARACTERES {
            blocs.push(Bloc { nature: "ligne", numero: debut as f64, texte: std::mem::take(&mut courant) });
        }
        if courant.is_empty() {
            debut = i + 1;
        }
        courant.push_str(ligne);
        courant.push('\n');
    }
    if !courant.is_empty() {
        blocs.push(Bloc { nature: "ligne", numero: debut as f64, texte: courant });
    }
    Ok(blocs)
}

/// UTF-8 ou UTF-16 (avec BOM). Refuse les fichiers binaires.
fn decoder(octets: &[u8]) -> Result<String> {
    match octets {
        [0xFF, 0xFE, reste @ ..] => Ok(String::from_utf16_lossy(
            &reste.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>(),
        )),
        [0xFE, 0xFF, reste @ ..] => Ok(String::from_utf16_lossy(
            &reste.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>(),
        )),
        _ => {
            if octets.iter().take(8192).any(|&b| b == 0) {
                bail!("fichier binaire");
            }
            let sans_bom = octets.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(octets);
            Ok(String::from_utf8_lossy(sans_bom).into_owned())
        }
    }
}

/// Retire balises, scripts et styles d'une page HTML.
fn sans_balises(html: &str) -> String {
    let mut sortie = String::with_capacity(html.len() / 2);
    let minuscule = html.to_ascii_lowercase();
    let mut i = 0;
    while i < html.len() {
        if html.as_bytes()[i] == b'<' {
            let ignore_jusqua = ["script", "style"]
                .into_iter()
                .find(|b| minuscule[i + 1..].starts_with(b))
                .and_then(|b| minuscule[i..].find(&format!("</{b}")).map(|p| i + p));
            let suite = ignore_jusqua.unwrap_or(i);
            i = minuscule[suite..].find('>').map(|p| suite + p + 1).unwrap_or(html.len());
            sortie.push(' ');
        } else {
            let fin = html[i..].find('<').map(|p| i + p).unwrap_or(html.len());
            sortie.push_str(&html[i..fin]);
            i = fin;
        }
    }
    sortie
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}
