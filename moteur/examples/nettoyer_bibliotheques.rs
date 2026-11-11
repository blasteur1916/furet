//! Une fois : retire de l'index le code des grandes bibliothèques lu par une version précédente (il reste
//! trouvable par son nom). Sans `--oublier`, ne fait que compter.
//! cargo run --release -p furet-moteur --example nettoyer_bibliotheques -- <index.sqlite> <dossier>... [--oublier]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let oublier = args.iter().any(|a| a == "--oublier");
    let index = PathBuf::from(&args[0]);
    let dossiers: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
    let mut base = furet_moteur::base::ouvrir(&index)?;
    let etats = furet_moteur::base::etats(&base)?;
    let mut ids = Vec::new();
    for d in dossiers {
        let en_bibliotheque: HashSet<PathBuf> = furet_moteur::indexation::parcours_detaille(Path::new(d))
            .into_iter()
            .filter(|f| f.3)
            .map(|f| f.0)
            .collect();
        ids.extend(etats.iter().filter(|(c, _)| en_bibliotheque.contains(Path::new(c.as_str()))).map(|(_, e)| e.id));
    }
    let morceaux: i64 = base.query_row("SELECT COUNT(*) FROM morceaux", [], |r| r.get(0))?;
    let a_retirer: i64 = if ids.is_empty() {
        0
    } else {
        let mut n = 0;
        for p in ids.chunks(500) {
            let liste = p.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            n += base.query_row(&format!("SELECT COUNT(*) FROM morceaux WHERE fichier_id IN ({liste})"), [], |r| r.get::<_, i64>(0))?;
        }
        n
    };
    println!("{} fichiers de l'index sont du code de grandes bibliothèques : {a_retirer} morceaux sur {morceaux}", ids.len());
    if oublier {
        let t = Instant::now();
        furet_moteur::base::oublier(&mut base, &ids)?;
        println!("retirés en {:.0} s", t.elapsed().as_secs_f64());
        let t = Instant::now();
        let n = alleger(&mut base)?;
        println!("{n} passages en trop (gros fichiers, archives) gardés pour leurs mots exacts, sans vecteur, en {:.0} s", t.elapsed().as_secs_f64());
        let t = Instant::now();
        let n = archives(&mut base)?;
        println!("{n} passages d'archives venus de code, de réglages ou de pages web retirés en {:.0} s", t.elapsed().as_secs_f64());
        let t = Instant::now();
        base.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        println!("index compacté en {:.0} s", t.elapsed().as_secs_f64());
    }
    Ok(())
}

/// Dans les archives, les passages venus de fichiers qu'on ne lit plus (`fichiers::de_personne_dans_archive`),
/// et tous ceux des archives de logiciels (`fichiers::archive_de_logiciel`).
fn archives(base: &mut rusqlite::Connection) -> anyhow::Result<usize> {
    let logiciels: Vec<i64> = {
        let mut req = base.prepare("SELECT id, chemin FROM fichiers WHERE genre = 'archive' AND nb_morceaux > 0")?;
        let toutes: Vec<(i64, String)> = req.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        toutes
            .into_iter()
            .filter(|(_, chemin)| {
                std::fs::File::open(chemin)
                    .ok()
                    .and_then(|f| zip::ZipArchive::new(f).ok())
                    .is_some_and(|z| furet_moteur::fichiers::archive_de_logiciel(z.file_names().filter_map(|n| n.ok())))
            })
            .map(|(id, _)| id)
            .collect()
    };
    println!("{} archives de logiciels : seulement par leur nom", logiciels.len());
    let ids: Vec<i64> = {
        let mut req = base.prepare(
            "SELECT m.id, m.partie FROM morceaux m JOIN fichiers f ON f.id = m.fichier_id
             WHERE f.genre = 'archive' AND m.partie IS NOT NULL",
        )?;
        req.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, partie)| !furet_moteur::fichiers::de_personne_dans_archive(Path::new(partie)))
            .map(|(id, _)| id)
            .collect()
    };
    let mut ids = ids;
    for p in logiciels.chunks(500) {
        let liste = p.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let mut req = base.prepare(&format!("SELECT id FROM morceaux WHERE fichier_id IN ({liste})"))?;
        ids.extend(req.query_map([], |r| r.get::<_, i64>(0))?.filter_map(|r| r.ok()));
    }
    ids.sort_unstable();
    ids.dedup();
    let t = base.transaction()?;
    for p in ids.chunks(1000) {
        let liste = p.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        t.execute(&format!("DELETE FROM vecteurs WHERE rowid IN ({liste})"), [])?;
        // Les déclencheurs retirent aussi leur texte et préviennent l'index rapide.
        t.execute(&format!("DELETE FROM morceaux WHERE id IN ({liste})"), [])?;
    }
    t.execute(
        "UPDATE fichiers SET nb_morceaux = (SELECT COUNT(*) FROM morceaux WHERE fichier_id = fichiers.id) WHERE genre = 'archive'",
        [],
    )?;
    t.commit()?;
    Ok(ids.len())
}

/// Les règles de `fichiers::alleger` et des archives, appliquées à ce qui est déjà dans l'index : les
/// passages en trop perdent leur vecteur (ils gardent leur texte, donc leurs mots exacts).
fn alleger(base: &mut rusqlite::Connection) -> anyhow::Result<usize> {
    const PLEINS: usize = 40;
    const PAS: usize = 4;
    const MAX: usize = 200;
    const ARCHIVE_MAX: usize = 300;
    let lignes: Vec<(i64, String, i64, bool)> = {
        let mut req = base.prepare(
            "SELECT m.fichier_id, f.genre, m.id, v.rowid IS NOT NULL FROM morceaux m JOIN fichiers f ON f.id = m.fichier_id
             JOIN textes t ON t.rowid = m.id LEFT JOIN vecteurs_rowids v ON v.rowid = m.id
             WHERE f.genre IN ('texte', 'code', 'tableur', 'archive') ORDER BY m.fichier_id, m.id",
        )?;
        req.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let mut retirer = Vec::new();
    let (mut fichier, mut rang, mut gardes) = (-1, 0, 0);
    for (f, genre, id, vecteur) in lignes {
        if f != fichier {
            (fichier, rang, gardes) = (f, 0, 0);
        }
        rang += 1;
        let garder = if genre == "archive" {
            rang <= ARCHIVE_MAX
        } else {
            rang <= PLEINS || ((rang - PLEINS) % PAS == 0 && gardes < MAX)
        };
        if garder {
            gardes += 1;
        } else if vecteur {
            retirer.push(id);
        }
    }
    let t = base.transaction()?;
    for p in retirer.chunks(1000) {
        let liste = p.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        t.execute(&format!("DELETE FROM vecteurs WHERE rowid IN ({liste})"), [])?;
        t.execute(&format!("UPDATE morceaux SET biais = NULL WHERE id IN ({liste})"), [])?;
        // L'index rapide doit les oublier aussi : le journal le lui dit.
        t.execute(&format!("INSERT INTO journal_vecteurs (id, op) SELECT value, 0 FROM json_each('[{liste}]')"), [])?;
    }
    t.commit()?;
    Ok(retirer.len())
}
