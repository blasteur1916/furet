import { invoke } from "@tauri-apps/api/core";

/** La langue de l'interface : celle du réglage, sinon celle de Windows (voir `reglages.rs`). */
export let langue: "fr" | "en" = "en";

/** À appeler une fois, avant d'afficher quoi que ce soit. */
export async function chargerLangue(): Promise<void> {
  try {
    langue = (await invoke<string>("langue")) === "fr" ? "fr" : "en";
  } catch {
    // Hors de l'appli (aperçu dans un navigateur) : l'anglais.
  }
  document.documentElement.lang = langue;
}

/** Le texte dans la langue de l'interface. */
export const tr = (fr: string, en: string): string => (langue === "fr" ? fr : en);

/** « 1 240 » ou « 1,240 ». */
export const nombre = (n: number, options?: Intl.NumberFormatOptions): string =>
  n.toLocaleString(langue === "fr" ? "fr-FR" : "en-US", options);
