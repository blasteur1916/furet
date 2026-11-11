//! Furet, prototype de mesure : indexe un dossier avec EmbeddingGemma 2 et répond à des recherches.
//! N'affiche jamais le contenu des fichiers : seulement le chemin, le score et la position.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use furet_moteur::fichiers::Genre;
use furet_moteur::mesures::{mo, secondes, taille_base};
use furet_moteur::serveur::{self, Serveur};
use furet_moteur::{base, filtres, indexation, pertinence};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "furet-proto", about = "Prototype de mesure de Furet (recherche par le contenu)")]
struct Cli {
    /// Dossier du projet (outils, modeles)
    #[arg(long, global = true, env = "FURET_RACINE", default_value = r"D:\furet")]
    racine: PathBuf,
    /// Fichier de l'index (par défaut : <racine>\donnees\index.sqlite)
    #[arg(long, global = true)]
    base: Option<PathBuf>,
    #[command(subcommand)]
    commande: Commande,
}

#[derive(Subcommand)]
enum Commande {
    /// Indexe un dossier : photos, PDF, audio, vidéo, documents, tableurs, texte et code
    Index {
        dossier: PathBuf,
        /// Forcer le processeur au lieu de la carte graphique
        #[arg(long)]
        cpu: bool,
        /// Tout réindexer, même les fichiers inchangés
        #[arg(long)]
        tout: bool,
        /// Ne pas lire le texte des images et des scans (OCR)
        #[arg(long)]
        sans_ocr: bool,
        /// Ne pas transcrire la parole des audios et des vidéos
        #[arg(long)]
        sans_transcription: bool,
        /// Ne pas reconnaître les visages des photos
        #[arg(long)]
        sans_visages: bool,
        /// Fils qui préparent les fichiers en parallèle (0 : automatique)
        #[arg(long, default_value_t = 0)]
        fils: usize,
        /// Jetons par image pour le modèle (70, 140, 280… : plus = plus fin mais plus lent).
        /// Par défaut : 140 sur carte graphique, 70 sur processeur
        #[arg(long)]
        jetons_image: Option<usize>,
    },
    /// Cherche dans l'index (sans texte : mode interactif)
    Cherche {
        texte: Option<String>,
        /// Nombre de résultats
        #[arg(short, default_value_t = 10)]
        n: usize,
        /// Seulement un type : photo, pdf, audio, video, document, tableur, texte ou code
        #[arg(long = "type")]
        genre: Option<String>,
        #[arg(long)]
        cpu: bool,
        /// Score minimal des résultats par le contenu (0 pour tout voir)
        #[arg(long, default_value_t = pertinence::SEUIL)]
        seuil: f32,
    },
    /// Résume l'index : temps par fichier, mémoire, place prise
    Stats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    furet_moteur::processus::lier_au_programme();
    let chemin_base = cli.base.clone().unwrap_or_else(|| cli.racine.join("donnees").join("index.sqlite"));
    match cli.commande {
        Commande::Index { dossier, cpu, tout, sans_ocr, sans_transcription, sans_visages, fils, jetons_image } => {
            // Le prototype ne fait qu'indexer : tout le programme passe en priorité basse.
            furet_moteur::processus::programme_en_priorite_basse();
            indexation::indexer(
                &cli.racine,
                &chemin_base,
                &[dossier],
                indexation::Options {
                    cpu,
                    tout,
                    avec_ocr: !sans_ocr,
                    avec_transcription: !sans_transcription,
                    avec_visages: !sans_visages,
                    fils,
                    jetons_image,
                    arret: Default::default(),
                    suivi: None,
                },
            )
        }
        Commande::Cherche { texte, n, genre, cpu, seuil } => chercher(&cli.racine, &chemin_base, texte, n, genre, cpu, seuil),
        Commande::Stats => stats(&chemin_base),
    }
}

/// Résultats « par le nom » affichés au-dessus de ceux « par le contenu ».
const NOMS_MAX: usize = 5;

fn chercher(
    racine: &Path,
    chemin_base: &Path,
    texte: Option<String>,
    n: usize,
    genre: Option<String>,
    cpu: bool,
    seuil: f32,
) -> Result<()> {
    if let Some(g) = &genre {
        Genre::depuis_nom(g).with_context(|| format!("type inconnu « {g} » ({})", Genre::noms()))?;
    }
    let base = base::ouvrir(chemin_base)?;
    let serveur = Serveur::demarrer(racine, cpu, None, &chemin_base.with_file_name("serveur.log"))?;
    eprintln!("Modèle chargé sur : {} (en {})", serveur.appareil, secondes(serveur.demarrage.as_secs_f64() * 1000.0));
    let charabia = pertinence::Charabia::charger(&serveur.client())?;

    let une = |q: &str| -> Result<()> {
        let t = Instant::now();
        let par_nom = base::chercher_noms(&base, q, NOMS_MAX, genre.as_deref())?;
        let ms_noms = t.elapsed().as_secs_f64() * 1000.0;
        // Les filtres tapés (« photos 2024 ») s'ajoutent à --type.
        let analyse = filtres::analyser(q);
        let mut filtre = analyse.filtre.clone();
        if let Some(g) = genre.as_deref().and_then(Genre::depuis_nom) {
            filtre.genres = vec![g.nom()];
        }
        if !analyse.etiquettes.is_empty() {
            println!("  Filtres : {} ; texte : « {} »", analyse.etiquettes.join(", "), analyse.reste);
        }
        let q = analyse.reste.as_str();
        if q.is_empty() {
            for r in base::lister(&base, &filtre, n)? {
                println!("    {:<8} {:>9}  {}  {}", r.genre, filtres::taille_lisible(r.taille), filtres::date_lisible(r.modifie), r.chemin);
            }
            return Ok(());
        }
        let t = Instant::now();
        let v = serveur.vecteur_requete(q)?;
        let ms_modele = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        // Le seuil donné, relevé pour une recherche qui ressemble à des mots inventés.
        let seuil = if seuil > 0.0 { seuil.max(charabia.seuil(&v)) } else { seuil };
        let requete = base::Requete { vecteur: &v, texte: q, n, filtre: &filtre, seuil };
        let (resultats, ecartes) = base::chercher(&base, &requete, None)?;
        let ms_base = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "« {q} »  (noms {ms_noms:.0} ms ; contenu : phrase → vecteur {ms_modele:.0} ms, recherche {ms_base:.0} ms)"
        );
        if !par_nom.is_empty() {
            println!("  Par le nom :");
            for (score, chemin) in par_nom {
                println!("    {score:.2}  {chemin}");
            }
        }
        let personnes = furet_moteur::visages::chercher_personne(&base, q, n)?;
        if !personnes.is_empty() {
            println!("  Visage reconnu :");
            for p in personnes {
                println!("    {:+.3}  {:<16} {}", p.ressemblance, p.prenom, p.chemin);
            }
        }
        println!("  Par le contenu :");
        if resultats.is_empty() {
            println!("    aucun résultat assez proche");
        }
        for r in resultats {
            // L'extrait (texte du fichier) n'est pas affiché ici : seulement s'il a été trouvé par ses mots.
            let mots = if r.exact { "[mots]" } else { "" };
            println!("    {:.3} {mots:<6} {:<8} {:<22} {}", r.score, r.genre, r.position(), r.chemin);
        }
        if ecartes > 0 {
            println!("    ({ecartes} fichier(s) moins proche(s) masqué(s) ; --seuil 0 pour tout voir)");
        }
        Ok(())
    };

    match texte {
        Some(q) => une(&q)?,
        None => {
            eprintln!("Tape une recherche (ligne vide pour quitter).");
            let stdin = std::io::stdin();
            loop {
                print!("> ");
                std::io::stdout().flush()?;
                let mut ligne = String::new();
                if stdin.lock().read_line(&mut ligne)? == 0 {
                    break;
                }
                let ligne = ligne.trim().trim_start_matches('\u{feff}');
                if ligne.is_empty() {
                    break;
                }
                if let Err(e) = une(ligne) {
                    println!("  erreur : {e:#}");
                }
            }
        }
    }
    Ok(())
}

fn stats(chemin_base: &Path) -> Result<()> {
    let base = base::ouvrir(chemin_base)?;
    println!("Index : {}\n", chemin_base.display());
    println!(
        "{:<8} {:>8} {:>9} {:>8} {:>13} {:>10} {:>13} {:>12}",
        "type", "fichiers", "morceaux", "erreurs", "moyenne/fich.", "dont OCR", "pire fichier", "pages scan"
    );
    let mut req = base.prepare(
        "SELECT genre, SUM(erreur IS NULL), SUM(nb_morceaux), SUM(erreur IS NOT NULL),
                AVG(CASE WHEN erreur IS NULL THEN prep_ms + modele_ms END),
                MAX(CASE WHEN erreur IS NULL THEN prep_ms + modele_ms END), SUM(pages_sans_texte),
                AVG(CASE WHEN erreur IS NULL THEN COALESCE(ocr_ms, 0) END)
         FROM fichiers GROUP BY genre ORDER BY genre",
    )?;
    let lignes = req.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
            r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
            r.get::<_, i64>(6)?,
            r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
        ))
    })?;
    let mut morceaux_total = 0;
    for l in lignes {
        let (g, f, m, e, moy, pire, scans, ocr) = l?;
        morceaux_total += m;
        println!(
            "{g:<8} {f:>8} {m:>9} {e:>8} {:>13} {:>10} {:>13} {scans:>12}",
            secondes(moy), secondes(ocr), secondes(pire)
        );
    }

    println!("\nDernières indexations :");
    let mut req = base.prepare(
        "SELECT date, appareil, nb_fichiers, duree_s, demarrage_s, ram_programme, ram_serveur, vram
         FROM sessions ORDER BY id DESC LIMIT 5",
    )?;
    let sessions = req.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, f64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, Option<i64>>(7)?,
        ))
    })?;
    for s in sessions {
        let (date, app, n, duree, dem, rp, rs, vram) = s?;
        let vram = vram.map(|v| format!(", carte graphique +{}", mo(v as u64))).unwrap_or_default();
        println!(
            "  {date}  {app} : {n} fichier(s) en {}, démarrage {}, mémoire programme {} / llama-server {}{vram}",
            secondes(duree * 1000.0), secondes(dem * 1000.0), mo(rp as u64), mo(rs as u64)
        );
    }

    let taille = taille_base(chemin_base);
    let vecteurs = morceaux_total as u64 * serveur::DIMENSION_INDEX as u64 * 4;
    println!("\nNoms de fichiers enregistrés (tous types) : {}", base::nombre_noms(&base)?);
    println!("Place de l'index : {} (dont vecteurs bruts {})", mo(taille), mo(vecteurs));
    Ok(())
}
