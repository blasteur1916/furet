//! Lance llama-server en arrière-plan et lui demande les vecteurs.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::fs::File;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

pub const DIMENSION: usize = 768;
/// On ne garde que les 256 premières dimensions (Matryoshka) : index 3 fois plus petit, recherche 3 fois
/// plus rapide, et sur nos tests le classement est même un peu meilleur une fois les « aimants » corrigés.
pub const DIMENSION_INDEX: usize = 256;
/// Jetons par image par défaut : 140 est 1,5 fois plus rapide que 280, sans perte visible sur nos tests.
pub const JETONS_IMAGE_DEFAUT: usize = 140;
/// Sur le processeur, 70 jetons : environ 2,7 fois plus rapide qu'à 280, sans perte visible sur nos tests.
pub const JETONS_IMAGE_PROCESSEUR: usize = 70;

pub struct Serveur {
    processus: Child,
    client: Client,
    pub appareil: String,
    pub demarrage: Duration,
}

/// Ce qui parle au serveur : se copie pour envoyer des requêtes depuis plusieurs fils.
#[derive(Clone)]
pub struct Client {
    adresse: String,
    agent: ureq::Agent,
}

impl Serveur {
    /// Démarre sur la meilleure carte graphique Vulkan, sinon sur le processeur.
    /// Jetons par image : `jetons_image` s'il est donné, sinon 140 sur carte graphique et 70 sur processeur.
    pub fn demarrer(racine: &Path, forcer_cpu: bool, jetons_image: Option<usize>, journal: &Path) -> Result<Self> {
        if !forcer_cpu {
            let jetons = jetons_image.unwrap_or(JETONS_IMAGE_DEFAUT);
            match choisir_carte(racine) {
                Some((id, nom)) => match Self::lancer_indexation(racine, Some(&id), &nom, jetons, journal) {
                    Ok(s) => return Ok(s),
                    Err(e) => eprintln!("Carte graphique indisponible ({e:#}), passage au processeur."),
                },
                None => eprintln!("Aucune carte graphique Vulkan trouvée, passage au processeur."),
            }
        }
        Self::lancer_indexation(racine, None, "processeur", jetons_image.unwrap_or(JETONS_IMAGE_PROCESSEUR), journal)
    }

    /// Serveur léger pour les recherches de l'appli : le modèle texte seul, sur le processeur.
    /// Mesuré : prêt en 1,5 s, 38 ms par recherche, 274 Mo de mémoire, et la carte graphique reste au repos
    /// (la carte graphique ne ferait gagner que 9 ms, mais resterait éveillée : batterie).
    pub fn demarrer_requetes(racine: &Path, journal: &Path) -> Result<Self> {
        let coeurs = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).clamp(1, 4);
        let modele = racine.join("modeles").join("embeddinggemma-2-Q8_0.gguf");
        let args = [
            "-m", &modele.to_string_lossy(),
            "--embeddings", "-c", "512", "-b", "512", "-ub", "512", "-np", "1", "-t", &coeurs.to_string(),
            "--no-webui",
        ]
        .map(String::from)
        .to_vec();
        Self::lancer(racine, "llama-cpu", "processeur (recherche)", args, Duration::from_secs(10), false, journal)
    }

    fn lancer_indexation(racine: &Path, carte: Option<&str>, nom: &str, jetons_image: usize, journal: &Path) -> Result<Self> {
        let modeles = racine.join("modeles");
        let mut args: Vec<String> = [
            "-m", &modeles.join("embeddinggemma-2-Q8_0.gguf").to_string_lossy(),
            "--mmproj", &modeles.join("mmproj-embeddinggemma-2-Q8_0.gguf").to_string_lossy(),
            "--embeddings", "-c", "8192", "-b", "8192", "-ub", "8192",
            "--image-max-tokens", &jetons_image.to_string(),
            "--no-webui",
        ]
        .map(String::from)
        .to_vec();
        if let Some(id) = carte {
            args.extend(["--device".into(), id.into(), "-ngl".into(), "99".into()]);
        }
        let dossier = if carte.is_some() { "llama-vulkan" } else { "llama-cpu" };
        Self::lancer(racine, dossier, nom, args, Duration::from_secs(900), true, journal)
    }

    fn lancer(
        racine: &Path,
        dossier: &str,
        nom: &str,
        mut args: Vec<String>,
        delai: Duration,
        basse: bool,
        journal: &Path,
    ) -> Result<Self> {
        let exe = racine.join("outils").join(dossier).join("llama-server.exe");
        let port = port_libre()?;
        args.extend(["--host".into(), "127.0.0.1".into(), "--port".into(), port.to_string()]);

        let log = File::create(journal).context("création du journal du serveur")?;
        let debut = Instant::now();
        let mut commande = if basse { crate::processus::commande_basse(&exe) } else { crate::processus::commande(&exe) };
        let processus = commande
            .args(&args)
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| format!("lancement de {}", exe.display()))?;

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(delai))
            .http_status_as_error(false)
            .build()
            .into();
        let mut serveur = Serveur {
            processus,
            client: Client { adresse: format!("http://127.0.0.1:{port}"), agent },
            appareil: nom.to_string(),
            demarrage: Duration::ZERO,
        };
        serveur.attendre_pret()?;
        serveur.demarrage = debut.elapsed();
        Ok(serveur)
    }

    fn attendre_pret(&mut self) -> Result<()> {
        let debut = Instant::now();
        loop {
            if let Some(code) = self.processus.try_wait()? {
                bail!("llama-server s'est arrêté ({code}), voir le journal");
            }
            if let Ok(mut r) = self.client.agent.get(format!("{}/health", self.client.adresse)).call() {
                if r.status() == 200 {
                    let v: Value = r.body_mut().read_json().unwrap_or_default();
                    if v["status"] == "ok" {
                        return Ok(());
                    }
                }
            }
            if debut.elapsed() > Duration::from_secs(300) {
                bail!("llama-server n'est pas prêt après 5 minutes");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub fn vecteur_requete(&self, texte: &str) -> Result<Vec<f32>> {
        self.client.vecteur_requete(texte)
    }

    /// Pic de mémoire vive de llama-server, en octets.
    pub fn memoire_max(&self) -> u64 {
        use std::os::windows::io::AsRawHandle;
        crate::mesures::pic_memoire(windows::Win32::Foundation::HANDLE(self.processus.as_raw_handle()))
    }
}

impl Client {
    /// Le vecteur d'une recherche, avec le préfixe « requête » de EmbeddingGemma.
    pub fn vecteur_requete(&self, texte: &str) -> Result<Vec<f32>> {
        let entree = Value::String(format!("task: search result | query: {texte}"));
        Ok(self.vecteurs(vec![entree])?.remove(0))
    }

    /// Un vecteur normalisé par entrée (texte simple ou objet {content: [...]}).
    pub fn vecteurs(&self, entrees: Vec<Value>) -> Result<Vec<Vec<f32>>> {
        let n = entrees.len();
        let mut r = self
            .agent
            .post(format!("{}/v1/embeddings", self.adresse))
            .send_json(json!({ "input": entrees }))?;
        let statut = r.status();
        if statut != 200 {
            let texte = r.body_mut().read_to_string().unwrap_or_default();
            bail!("llama-server a répondu {statut} : {}", texte.chars().take(300).collect::<String>());
        }
        let v: Value = r.body_mut().with_config().limit(256 << 20).read_json()?;
        let mut sortie = vec![Vec::new(); n];
        for d in v["data"].as_array().ok_or_else(|| anyhow!("réponse sans data"))? {
            let i = d["index"].as_u64().unwrap_or(0) as usize;
            let mut e: Vec<f32> = d["embedding"]
                .as_array()
                .ok_or_else(|| anyhow!("réponse sans embedding"))?
                .iter()
                .map(|x| x.as_f64().unwrap_or(0.0) as f32)
                .collect();
            if e.len() != DIMENSION {
                bail!("dimension inattendue : {}", e.len());
            }
            e.truncate(DIMENSION_INDEX);
            let norme = e.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
            e.iter_mut().for_each(|x| *x /= norme);
            if i < n {
                sortie[i] = e;
            }
        }
        if sortie.iter().any(|e| e.is_empty()) {
            bail!("il manque des vecteurs dans la réponse");
        }
        Ok(sortie)
    }
}

impl Drop for Serveur {
    fn drop(&mut self) {
        let _ = self.processus.kill();
        let _ = self.processus.wait();
    }
}

fn port_libre() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Lit `llama-server --list-devices` et garde la carte dédiée avec le plus de mémoire.
fn choisir_carte(racine: &Path) -> Option<(String, String)> {
    let exe = racine.join("outils").join("llama-vulkan").join("llama-server.exe");
    let sortie = crate::processus::commande(exe).arg("--list-devices").output().ok()?;
    let texte = String::from_utf8_lossy(&sortie.stdout).to_string() + &String::from_utf8_lossy(&sortie.stderr);
    let mut cartes: Vec<(bool, u64, String, String)> = texte
        .lines()
        .filter_map(|l| {
            let (id, reste) = l.trim().split_once(": ")?;
            if !id.starts_with("Vulkan") {
                return None;
            }
            let (nom, mem) = reste.rsplit_once(" (")?;
            let mio = mem.split_whitespace().next()?.parse().ok()?;
            let integree = nom.contains("Intel") && !nom.contains("Arc");
            Some((!integree, mio, id.to_string(), nom.to_string()))
        })
        .collect();
    cartes.sort();
    cartes.pop().map(|(_, _, id, nom)| (id, nom))
}
