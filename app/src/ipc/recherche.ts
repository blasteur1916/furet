import { invoke } from "@tauri-apps/api/core";

/** Miroir de `Ligne` dans `src-tauri/src/recherche.rs`. */
export interface Ligne {
  /** Ce qui désigne le fichier pour l'ouvrir. */
  cle: string;
  nom: string;
  dossier: string;
  genre: "photo" | "pdf" | "audio" | "video" | "document" | "tableur" | "texte" | "code" | "archive" | "dossier" | "fichier";
  /** « page 4 », « dit 01:23–01:53 »… vide pour les résultats par le nom. */
  position: string;
  /** L'adresse de sa miniature Windows. */
  miniature: string;
  /** Le passage où ses mots ont été trouvés : les mots sont entourés de \u0001 et \u0002. */
  extrait: string | null;
  /** La page, la ligne ou le moment (secondes) trouvé : l'aperçu s'y ouvre. */
  repere: number | null;
}

/** Miroir de `Filtres` dans `src-tauri/src/recherche.rs`. */
export interface Filtres {
  /** « Photos », « 2024 », « > 1 Go » */
  etiquettes: string[];
  /** Ce qui reste à chercher : vide s'il n'y a que des filtres. */
  reste: string;
}

/** Miroir de `EtatIndex` dans `src-tauri/src/recherche.rs`. */
export interface EtatIndex {
  /** Fichiers lus par leur contenu. */
  fichiers: number;
  noms: number;
  pret: boolean;
}

export const etatIndex = () => invoke<EtatIndex>("etat_index");
export const chercherNoms = (texte: string) => invoke<Ligne[]>("chercher_noms", { texte });
export const chercherContenu = (texte: string) => invoke<Ligne[]>("chercher_contenu", { texte });
export const lireFiltres = (texte: string) => invoke<Filtres>("filtres", { texte });
/** Les fichiers ouverts récemment depuis Furet (pour la barre vide). */
export const chercherRecents = () => invoke<Ligne[]>("recents");
/** L'ouvre, ou avec `dossier` le montre dans l'Explorateur. */
export const ouvrirFichier = (ligne: Ligne, dossier: boolean) =>
  invoke<void>("ouvrir_fichier", { cle: ligne.cle, dossier });
