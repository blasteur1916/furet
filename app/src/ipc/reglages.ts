import { invoke } from "@tauri-apps/api/core";

/** Miroir de `Reglages` dans `src-tauri/src/reglages.rs`. */
export interface Reglages {
  dossiers: string[];
  ocr: boolean;
  transcription: boolean;
  visages: boolean;
  /** Lire aussi le code des grandes bibliothèques (moteurs de jeu, SDK…). */
  bibliotheques: boolean;
  pause_batterie: boolean;
  pause_jeu: boolean;
  /** Lancer Furet avec Windows. */
  demarrage: boolean;
  /** « auto » (la langue de Windows), « en » ou « fr ». */
  langue: string;
}

/**
 * Vrai pendant que le sélecteur de dossier est ouvert : il prend le clavier, mais l'île ne doit pas
 * se fermer pour autant.
 */
export let dialogueOuvert = false;

export const lireReglages = () => invoke<Reglages>("reglages_lire");
export const ecrireReglages = (reglages: Reglages) => invoke<Reglages>("reglages_ecrire", { reglages });
export const retirerDossier = (chemin: string) => invoke<Reglages>("retirer_dossier", { chemin });

export async function ajouterDossier(): Promise<Reglages> {
  dialogueOuvert = true;
  try {
    return await invoke<Reglages>("ajouter_dossier");
  } finally {
    // Le clavier revient à l'île un instant après la fermeture du sélecteur.
    setTimeout(() => (dialogueOuvert = false), 300);
  }
}
