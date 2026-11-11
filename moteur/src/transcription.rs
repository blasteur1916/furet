//! Transcription de la parole avec Parakeet TDT v3 (sherpa-onnx), un serveur local qui reste chargé.

use anyhow::{Context, Result, bail};
use std::fs::File;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use tungstenite::Message;

const MODELE: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
const PROGRAMME: &str = r"sherpa-onnx\sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts\bin\sherpa-onnx-offline-websocket-server.exe";

pub struct Transcripteur {
    processus: Child,
    port: u16,
    pub demarrage: Duration,
}

/// Ce qui parle au serveur de transcription : se copie dans chaque fil de préparation.
#[derive(Clone, Copy)]
pub struct Client {
    port: u16,
}

impl Transcripteur {
    /// `requetes` transcriptions en parallèle, chacune sur `coeurs` cœurs (au-delà de 4, Parakeet ne va pas plus vite).
    pub fn demarrer(racine: &Path, requetes: usize, coeurs: usize, journal: &Path) -> Result<Self> {
        let exe = racine.join("outils").join(PROGRAMME);
        let m = racine.join("modeles").join(MODELE);
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let log = File::create(journal)?;
        let debut = Instant::now();
        let mut processus = crate::processus::commande_basse(&exe)
            .arg(format!("--port={port}"))
            .arg(format!("--num-work-threads={requetes}"))
            .arg(format!("--max-batch-size={requetes}"))
            .arg(format!("--num-threads={coeurs}"))
            .arg(format!("--encoder={}", m.join("encoder.int8.onnx").display()))
            .arg(format!("--decoder={}", m.join("decoder.int8.onnx").display()))
            .arg(format!("--joiner={}", m.join("joiner.int8.onnx").display()))
            .arg(format!("--tokens={}", m.join("tokens.txt").display()))
            .arg("--model-type=nemo_transducer")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .with_context(|| format!("lancement de {}", exe.display()))?;
        // Le port n'accepte les connexions qu'une fois le modèle chargé.
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if let Some(code) = processus.try_wait()? {
                bail!("le serveur de transcription s'est arrêté ({code})");
            }
            if debut.elapsed() > Duration::from_secs(120) {
                let _ = processus.kill();
                bail!("le serveur de transcription n'est pas prêt après 2 minutes");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(Transcripteur { processus, port, demarrage: debut.elapsed() })
    }

    pub fn client(&self) -> Client {
        Client { port: self.port }
    }
}

impl Drop for Transcripteur {
    fn drop(&mut self) {
        let _ = self.processus.kill();
        let _ = self.processus.wait();
    }
}

impl Client {
    /// Texte prononcé dans des échantillons mono 16 kHz.
    pub fn transcrire(&self, echantillons: &[i16]) -> Result<String> {
        let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}", self.port))?;
        // Protocole de sherpa-onnx : fréquence (i32), taille en octets (i32), puis les échantillons en f32.
        let mut message = Vec::with_capacity(8 + echantillons.len() * 4);
        message.extend_from_slice(&16_000i32.to_le_bytes());
        message.extend_from_slice(&((echantillons.len() * 4) as i32).to_le_bytes());
        for &s in echantillons {
            message.extend_from_slice(&(s as f32 / 32768.0).to_le_bytes());
        }
        ws.send(Message::binary(message))?;
        let reponse = ws.read()?.into_text()?;
        let _ = ws.send(Message::text("Done"));
        let _ = ws.close(None);
        let v: serde_json::Value = serde_json::from_str(&reponse).context("réponse de transcription illisible")?;
        Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
    }
}
