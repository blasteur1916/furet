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

const nombre = (n: number) => n.toLocaleString("fr-FR");

/** « Lecture : 1 240 fichiers sur 5 000 », « À jour »… */
export function decrire(e: EtatIndexation): string {
  switch (e.phase) {
    case "attente":
      return "Aucun dossier à indexer";
    case "parcours":
      return "Recherche des fichiers nouveaux ou modifiés…";
    case "lecture":
      return `Lecture : ${nombre(e.faits)} fichier${e.total > 1 ? "s" : ""} sur ${nombre(e.total)}`;
    case "visages":
      return `Reconnaissance des visages : ${nombre(e.faits)} photo${e.total > 1 ? "s" : ""} sur ${nombre(e.total)}`;
    case "transcription":
      return `Transcription de la parole : ${nombre(e.faits)} sur ${nombre(e.total)}`;
    case "pause":
      return `Indexation en pause (${e.raison})`;
    case "a_jour":
      return "Index à jour";
    case "erreur":
      return "L'indexation a rencontré une erreur";
  }
}
