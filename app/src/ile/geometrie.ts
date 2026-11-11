/**
 * La forme vers laquelle l'île tend : cachée au-dessus du
 * bord de l'écran, ou ouverte à la taille de la barre et de ses résultats.
 */
import { FLARE, silhouetteWidth } from "./shell";

export interface ShellTarget {
  width: number;
  height: number;
  topRadius: number;
  bottomRadius: number;
  /** 0 → 1 : les « flares » qui relient le corps au bord de l'écran. */
  flares: number;
  /** Opacité du noir. */
  fill: number;
  /** Décalage vertical : négatif, au-dessus du bord (cachée). */
  y: number;
}

/** La largeur de la barre. */
export const LARGEUR_BARRE = 640;
const RAYON = 28;

/** Fermée : une petite pastille remontée derrière le bord de l'écran ; elle en redescend à l'ouverture. */
export const FERMEE: ShellTarget = { width: 200, height: 34, topRadius: 0, bottomRadius: 17, flares: 1, fill: 1, y: -38 };

export function ouverte(hauteur: number): ShellTarget {
  return { width: LARGEUR_BARRE, height: hauteur, topRadius: 0, bottomRadius: RAYON, flares: 1, fill: 1, y: 0 };
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * La zone de fenêtre qu'une forme occupe, en pixels CSS : du bord de l'écran jusqu'à son bas, centrée,
 * avec une marge pour l'anticrénelage. Null si elle est entièrement hors de l'écran.
 */
export function targetBox(t: ShellTarget, windowWidth: number, pad = 2): Box | null {
  const bottom = t.y + t.height;
  if (bottom <= 0) return null;
  const width = silhouetteWidth(t.width) + 2 * pad;
  return { x: Math.floor((windowWidth - width) / 2), y: 0, width: Math.ceil(width), height: Math.ceil(bottom + pad) };
}

export { FLARE };
