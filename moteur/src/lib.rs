//! Le moteur de Furet : indexe les fichiers par leur contenu (EmbeddingGemma 2, OCR, transcription)
//! et par leur nom, et y cherche. Utilisé par l'appli et par le prototype en ligne de commande.
//! N'écrit jamais le contenu des fichiers dans les journaux : seulement des chemins, des scores, des temps.

pub mod base;
pub mod disque;
pub mod documents;
pub mod fichiers;
pub mod filtres;
pub mod indexation;
pub mod installation;
pub mod langue;
pub mod lieux;
pub mod mesures;
pub mod noms;
pub mod ocr;
pub mod paroles;
pub mod pertinence;
pub mod photo;
pub mod processus;
pub mod serveur;
pub mod transcription;
pub mod vecteurs;
pub mod visages;
pub mod video;
