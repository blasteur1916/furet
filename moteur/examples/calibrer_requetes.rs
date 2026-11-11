//! Essai : les scores d'une recherche qui ne veut rien dire se détachent-ils moins du reste que ceux d'une
//! vraie recherche ? N'affiche que des nombres et des types de fichiers (jamais le contenu).
//! cargo run --release -p furet-moteur --example calibrer_requetes

use std::collections::HashMap;
use std::path::Path;

use furet_moteur::{base, pertinence, serveur::Serveur};
use rusqlite::params;

fn main() -> anyhow::Result<()> {
    let racine = Path::new(r"D:\furet");
    let b = base::ouvrir(&racine.join("donnees").join("index.sqlite"))?;
    let serveur = Serveur::demarrer_requetes(racine, &std::env::temp_dir().join("furet-calibrer.log"))?;
    let client = serveur.client();
    let requetes = [
        ("absurde", "zorglub"),
        ("absurde", "xkqzv"),
        ("absurde", "blorfique"),
        ("absurde", "qwfpgj"),
        ("absurde", "flibbertigibbet"),
        ("absurde", "trucmuche bidule"),
        ("vraie", "facture"),
        ("vraie", "photo d'un chat"),
        ("vraie", "coucher de soleil"),
        ("vraie", "capture d'écran"),
        ("vraie", "musique"),
        ("vraie", "personne qui parle"),
        ("vraie", "voiture"),
        ("vraie", "code python"),
        ("vraie", "recette de cuisine"),
        ("vraie", "montagne"),
        ("vraie", "jeu vidéo"),
        ("vraie", "logo"),
        ("vraie", "azerty"),
    ];
    // Des mots inventés (aucun n'est dans la liste des essais) : leur vecteur moyen.
    let inventes = ["bvnrtq", "plimzork", "gruntfax", "wexvoli", "krazhunf", "dofmipe", "snarglepuff", "trulvex", "yorpik", "zantibule", "mubrek", "fliznop"];
    let mut proto = vec![0f32; 256];
    for m in inventes {
        for (p, x) in proto.iter_mut().zip(client.vecteur_requete(m)?) {
            *p += x;
        }
    }
    let n = proto.iter().map(|x| x * x).sum::<f32>().sqrt();
    proto.iter_mut().for_each(|x| *x /= n);
    for (sorte, q) in requetes {
        let v = client.vecteur_requete(q)?;
        let charabia: f32 = v.iter().zip(&proto).map(|(a, b)| a * b).sum();
        print!("charabia {charabia:.3}  ");
        let octets: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        let mut req = b.prepare(
            "SELECT f.chemin, f.genre, v.distance, m.biais FROM vecteurs v JOIN morceaux m ON m.id = v.rowid
             JOIN fichiers f ON f.id = m.fichier_id WHERE v.embedding MATCH ?1 AND k = 400",
        )?;
        let mut par_fichier: HashMap<String, (f32, String)> = HashMap::new();
        for l in req.query_map(params![octets], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)? as f32, r.get::<_, Option<f64>>(3)?))
        })? {
            let (chemin, genre, d, biais) = l?;
            let s = pertinence::score(1.0 - d, biais.map(|x| x as f32));
            let e = par_fichier.entry(chemin).or_insert((f32::MIN, genre));
            e.0 = e.0.max(s);
        }
        let mut scores: Vec<(f32, String)> = par_fichier.into_values().collect();
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        let s: Vec<f32> = scores.iter().map(|x| x.0).collect();
        let n = s.len().min(100);
        let moyenne = s[..n].iter().sum::<f32>() / n as f32;
        let ecart = (s[..n].iter().map(|x| (x - moyenne).powi(2)).sum::<f32>() / n as f32).sqrt();
        let genres: Vec<&str> = scores.iter().take(3).map(|x| x.1.as_str()).collect();
        println!(
            "{sorte:<8} {q:<20} top {:.3}  5e {:.3}  20e {:.3}  moy100 {:.3}  écart {:.4}  z(top) {:+.2}  marge top-20e {:.3}  {:?}",
            s[0],
            s.get(4).copied().unwrap_or(0.0),
            s.get(19).copied().unwrap_or(0.0),
            moyenne,
            ecart,
            (s[0] - moyenne) / ecart.max(1e-6),
            s[0] - s.get(19).copied().unwrap_or(0.0),
            genres
        );
    }
    Ok(())
}
