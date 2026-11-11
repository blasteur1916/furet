import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Un rectangle en pixels CSS de la fenêtre de l'île. */
export interface RegionRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Limite le dessin et les clics de l'île à `rects` (vide : invisible et traversée par les clics). */
export function setIslandRegion(rects: RegionRect[]): Promise<void> {
  return invoke("ile_region", { rects });
}

/** Île fermée : WebView2 garde sa mémoire basse. */
export function setMemoireBasse(basse: boolean): void {
  invoke("ile_memoire", { basse }).catch(() => {});
}

/** Miroir de `Ouverture` dans `src-tauri/src/raccourci.rs`. */
export interface Ouverture {
  /** Texte de départ (mode de test). */
  texte: string | null;
  /** Mode de test : la barre ne se ferme pas quand elle perd le clavier. */
  test: boolean;
  /** Ouvrir directement les réglages (mode de test). */
  reglages: boolean;
  /** Ouvrir l'aperçu de ce résultat dès qu'il est là (mode de test). */
  apercu: number | null;
  /** Les outils et le modèle ne sont pas encore là : l'accueil à la place de la barre. */
  accueil: boolean;
}

/** Le raccourci ouvre la barre ; elle se ferme (Échap, clic ailleurs, fichier ouvert). */
export function onBarre(ouvrir: (o: Ouverture) => void, fermer: () => void): Promise<UnlistenFn> {
  return Promise.all([
    listen<Ouverture>("furet://ouvrir", (e) => ouvrir(e.payload)),
    listen("furet://fermer", fermer),
  ]).then((stops) => () => stops.forEach((stop) => stop()));
}

/** `rendre` : redonne le clavier à la fenêtre d'avant. */
export const fermerBarre = (rendre: boolean) => invoke<void>("fermer_barre", { rendre });
