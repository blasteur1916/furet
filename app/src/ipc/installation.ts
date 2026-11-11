import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { nombre, tr } from "../i18n";

/** Miroir de `Etat` dans `src-tauri/src/installation.rs`. */
export type EtatInstallation =
  | { phase: "pret" }
  /** Octets à télécharger pour l'essentiel, la transcription et les visages. */
  | { phase: "a_installer"; essentiel: number; transcription: number; visages: number }
  | { phase: "telechargement"; fait: number; total: number; nom: string }
  | { phase: "erreur"; message: string };

export const etatInstallation = () => invoke<EtatInstallation>("etat_installation");
export const installer = () => invoke<void>("installer");
export const raccourciActuel = () => invoke<string>("raccourci_actuel");
export const installerMiseAJour = () => invoke<void>("installer_mise_a_jour");

/** La version de Furet téléchargée et prête à être installée, s'il y en a une. */
export function useMiseAJour(): string | null {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let vivant = true;
    let arreter: UnlistenFn | undefined;
    invoke<string | null>("mise_a_jour_prete")
      .then((v) => vivant && setVersion(v))
      .catch(() => {});
    listen<string>("furet://mise-a-jour", (e) => setVersion(e.payload))
      .then((stop) => (vivant ? (arreter = stop) : stop()))
      .catch(() => {});
    return () => {
      vivant = false;
      arreter?.();
    };
  }, []);
  return version;
}

/** L'état de l'installation, tenu à jour. */
export function useInstallation(): EtatInstallation | null {
  const [etat, setEtat] = useState<EtatInstallation | null>(null);
  useEffect(() => {
    let vivant = true;
    let arreter: UnlistenFn | undefined;
    etatInstallation()
      .then((e) => vivant && setEtat(e))
      .catch(() => {});
    listen<EtatInstallation>("furet://installation", (e) => setEtat(e.payload))
      .then((stop) => (vivant ? (arreter = stop) : stop()))
      .catch(() => {});
    return () => {
      vivant = false;
      arreter?.();
    };
  }, []);
  return etat;
}

/** « 1,1 Go », « 340 Mo » */
export function taille(octets: number): string {
  if (octets >= 1e9) return `${nombre(octets / 1e9, { maximumFractionDigits: 1 })} ${tr("Go", "GB")}`;
  return `${nombre(Math.max(1, Math.round(octets / 1e6)))} ${tr("Mo", "MB")}`;
}
