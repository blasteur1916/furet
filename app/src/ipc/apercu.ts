import { invoke } from "@tauri-apps/api/core";
import type { Ligne } from "./recherche";

/** Miroir de `Contenu` dans `src-tauri/src/apercus.rs`. */
export type Contenu =
  | { type: "image"; adresse: string }
  | { type: "video"; adresse: string; debut: number; affiche: string }
  | { type: "audio"; adresse: string; debut: number; affiche: string }
  /** `adresse` + numéro de page donne l'image de la page. */
  | { type: "pdf"; adresse: string; page: number; pages: number }
  | { type: "texte"; blocs: BlocTexte[]; cible: number; lignes: boolean }
  | { type: "dossier"; entrees: { nom: string; dossier: boolean }[]; total: number }
  | { type: "miniature"; adresse: string };

export interface BlocTexte {
  /** « diapo 3 », « feuille 2 », ou vide pour des lignes. */
  titre: string;
  /** Le numéro de la première ligne (ou de la diapo, de la feuille). */
  numero: number;
  texte: string;
}

/** Miroir de `Apercu` dans `src-tauri/src/apercus.rs`. */
export interface Apercu {
  /** « 2,3 Mo · modifié le 3 mars 2024 » */
  infos: string;
  contenu: Contenu;
}

/** Les aperçus déjà demandés : celui du résultat choisi est préparé avant qu'on appuie sur Espace. */
const prets = new Map<string, Promise<Apercu>>();
const PRETS_MAX = 40;

export function lireApercu(ligne: Ligne): Promise<Apercu> {
  const cle = `${ligne.cle}|${ligne.repere ?? ""}`;
  let a = prets.get(cle);
  if (!a) {
    a = invoke<Apercu>("apercu", { cle: ligne.cle, repere: ligne.repere });
    // Un échec n'est pas gardé : on réessaiera.
    a.catch(() => prets.delete(cle));
    prets.set(cle, a);
    if (prets.size > PRETS_MAX) prets.delete(prets.keys().next().value as string);
  }
  return a;
}

/** Prépare l'aperçu d'un résultat, et son image, pour qu'il s'affiche aussitôt. */
export function preparerApercu(ligne: Ligne): void {
  lireApercu(ligne)
    .then(({ contenu: c }) => {
      const image = c.type === "image" ? c.adresse : c.type === "pdf" ? `${c.adresse}${c.page}` : c.type === "miniature" ? c.adresse : null;
      if (image) new Image().src = image;
    })
    .catch(() => {});
}

/** À l'ouverture de la barre : les fichiers ont pu changer depuis. */
export const oublierApercus = () => prets.clear();
export const copierChemin = (ligne: Ligne) => invoke<void>("copier_chemin", { cle: ligne.cle });
export const ouvrirAvec = (ligne: Ligne) => invoke<void>("ouvrir_avec", { cle: ligne.cle });
/** À appeler pendant que le bouton de la souris est enfoncé. */
export const glisser = (ligne: Ligne) => invoke<void>("glisser", { cle: ligne.cle });
