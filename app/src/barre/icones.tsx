import { useState } from "react";
import type { Ligne } from "../ipc/recherche";

const trait = { fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export function IconeRecherche() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" {...trait} strokeWidth={2.2} aria-hidden="true">
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </svg>
  );
}

/** La roue dentée des réglages. */
export function IconeReglages() {
  return (
    <svg width="17" height="17" viewBox="0 0 24 24" {...trait} strokeWidth={1.9} aria-hidden="true">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1Z" />
    </svg>
  );
}

export function IconeRetour() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" {...trait} strokeWidth={2.1} aria-hidden="true">
      <path d="M15 5.5 8.5 12l6.5 6.5" />
    </svg>
  );
}

/** Voir (l'aperçu). */
export function IconeOeil() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" {...trait} aria-hidden="true">
      <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z" />
      <circle cx="12" cy="12" r="2.8" />
    </svg>
  );
}

/** Montrer dans son dossier. */
export function IconeDossier() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" {...trait} aria-hidden="true">
      <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
    </svg>
  );
}

/** Copier le chemin. */
export function IconeCopier() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" {...trait} aria-hidden="true">
      <rect x="8.5" y="8.5" width="11" height="11" rx="2.5" />
      <path d="M15.5 8.5v-2a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2" />
    </svg>
  );
}

/** Ouvrir avec une autre appli. */
export function IconeOuvrirAvec() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" {...trait} aria-hidden="true">
      <path d="M13.5 4.5h6v6M19.5 4.5l-8 8M17.5 14v4a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h4" />
    </svg>
  );
}

/** Un pictogramme par type de fichier : affiché tant que la miniature n'est pas là, ou à sa place. */
export function Picto({ genre }: { genre: Ligne["genre"] }) {
  const dessin = (() => {
    switch (genre) {
      case "photo":
        return (
          <>
            <rect x="4" y="5" width="16" height="14" rx="2.5" />
            <circle cx="9" cy="10" r="1.6" />
            <path d="m5 17 4.5-4.5 3 3 2.5-2.5 4 4" />
          </>
        );
      case "audio":
        return (
          <>
            <path d="M9 17V6l10-2v11" />
            <circle cx="6.5" cy="17" r="2.5" />
            <circle cx="16.5" cy="15" r="2.5" />
          </>
        );
      case "video":
        return (
          <>
            <rect x="3.5" y="6" width="12.5" height="12" rx="2.5" />
            <path d="m16 10.5 4.5-2.5v8L16 13.5" />
          </>
        );
      case "tableur":
        return (
          <>
            <rect x="4" y="4.5" width="16" height="15" rx="2.5" />
            <path d="M4 9.5h16M4 14.5h16M10 4.5v15" />
          </>
        );
      case "code":
        return <path d="m8.5 7.5-4.5 4.5 4.5 4.5M15.5 7.5l4.5 4.5-4.5 4.5M13.5 5.5l-3 13" />;
      case "dossier":
        return <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />;
      default:
        return (
          <>
            <path d="M6.5 3.5h7l4.5 4.5v12.5h-11.5z" />
            <path d="M13.5 3.5V8h4.5" />
            {genre !== "fichier" && <path d="M9.5 12.5h5M9.5 16h5" />}
          </>
        );
    }
  })();
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" {...trait} aria-hidden="true">
      {dessin}
    </svg>
  );
}

/**
 * La miniature Windows du fichier (comme dans l'Explorateur). Le pictogramme du type reste dessous :
 * la miniature apparaît en fondu quand elle est prête, et s'il n'y en a pas, rien ne clignote.
 */
export function Miniature({ ligne }: { ligne: Ligne }) {
  const [prete, setPrete] = useState(false);
  const [absente, setAbsente] = useState(false);
  // Photos et vidéos remplissent le cadre ; les documents gardent leur page entière ou leur icône.
  const couvrir = ligne.genre === "photo" || ligne.genre === "video";
  return (
    <span className={`barre-mini barre-mini--${ligne.genre}`}>
      <Picto genre={ligne.genre} />
      {!absente && (
        <img
          className={`barre-mini-img ${couvrir ? "barre-mini-img--couvrir" : ""} ${prete ? "barre-mini-img--prete" : ""}`}
          src={ligne.miniature}
          alt=""
          draggable={false}
          decoding="async"
          onLoad={() => setPrete(true)}
          onError={() => setAbsente(true)}
        />
      )}
    </span>
  );
}
