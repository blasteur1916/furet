import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

/** Miroir de `Etat` dans `src-tauri/src/indexeur.rs`. */
export type EtatIndexation =
  | { phase: "attente" }
  | { phase: "parcours" }
  | { phase: "lecture"; faits: number; total: number }
  | { phase: "visages"; faits: number; total: number }
  | { phase: "transcription"; faits: number; total: number }
  | { phase: "pause"; raison: string }
  | { phase: "a_jour" }
  | { phase: "erreur"; message: string };

export const etatIndexation = () => invoke<EtatIndexation>("etat_indexation");
export const indexerMaintenant = () => invoke<void>("indexer_maintenant");
export const basculerPause = () => invoke<EtatIndexation>("basculer_pause");

export function onIndexation(rappel: (e: EtatIndexation) => void): Promise<UnlistenFn> {
  return listen<EtatIndexation>("furet://indexation", (e) => rappel(e.payload));
}

/** L'état de l'indexation, tenu à jour. */
export function useIndexation(): EtatIndexation {
  const [etat, setEtat] = useState<EtatIndexation>({ phase: "attente" });
  useEffect(() => {
    let vivant = true;
    let arreter: UnlistenFn | undefined;
    etatIndexation()
      .then((e) => vivant && setEtat(e))
      .catch(() => {});
    onIndexation(setEtat)
      .then((stop) => (vivant ? (arreter = stop) : stop()))
      .catch(() => {});
    return () => {
      vivant = false;
      arreter?.();
    };
  }, []);
  return etat;
}

import { nombre, tr } from "../i18n";

/** Pourquoi l'indexation est en pause (codes de `indexeur.rs`). */
const raisons: Record<string, [string, string]> = {
  manuelle: ["à la demande", "on request"],
  batterie: ["sur batterie", "on battery"],
  jeu: ["jeu ou plein écran", "game or full screen"],
};

/** « Lecture : 1 240 fichiers sur 5 000 », « À jour »… */
export function decrire(e: EtatIndexation): string {
  switch (e.phase) {
    case "attente":
      return tr("Aucun dossier à indexer", "No folder to index");
    case "parcours":
      return tr("Recherche des fichiers nouveaux ou modifiés…", "Looking for new or changed files…");
    case "lecture":
      return tr(
        `Lecture : ${nombre(e.faits)} fichier${e.total > 1 ? "s" : ""} sur ${nombre(e.total)}`,
        `Reading: ${nombre(e.faits)} of ${nombre(e.total)} file${e.total > 1 ? "s" : ""}`,
      );
    case "visages":
      return tr(
        `Reconnaissance des visages : ${nombre(e.faits)} photo${e.total > 1 ? "s" : ""} sur ${nombre(e.total)}`,
        `Recognizing faces: ${nombre(e.faits)} of ${nombre(e.total)} photo${e.total > 1 ? "s" : ""}`,
      );
    case "transcription":
      return tr(
        `Transcription de la parole : ${nombre(e.faits)} sur ${nombre(e.total)}`,
        `Transcribing speech: ${nombre(e.faits)} of ${nombre(e.total)}`,
      );
    case "pause":
      return tr(`Indexation en pause (${raisons[e.raison]?.[0] ?? e.raison})`, `Indexing paused (${raisons[e.raison]?.[1] ?? e.raison})`);
    case "a_jour":
      return tr("Index à jour", "Index up to date");
    case "erreur":
      return tr("L'indexation a rencontré une erreur", "Indexing ran into an error");
  }
}
