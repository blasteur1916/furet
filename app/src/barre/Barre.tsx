import { useEffect, useMemo, useRef, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { copierChemin, glisser, oublierApercus, ouvrirAvec, preparerApercu } from "../ipc/apercu";
import { fermerBarre } from "../ipc/ile";
import type { EtatIndexation } from "../ipc/indexation";
import { chercherContenu, chercherNoms, chercherRecents, etatIndex, lireFiltres, ouvrirFichier, type Filtres, type Ligne } from "../ipc/recherche";
import { Anneau } from "./Anneau";
import { Apercu, HAUTEUR_APERCU } from "./Apercu";
import { IconeCopier, IconeDossier, IconeOeil, IconeOuvrirAvec, IconeRecherche, IconeReglages, Miniature } from "./icones";
import { dossierCourt, surligner } from "./texte";
import "./barre.css";

const ignorer = () => {};
/** La recherche par le contenu part quand la frappe marque une pause de cette durée. */
const PAUSE_CONTENU_MS = 90;
/** Au-delà, un petit signe dit que la recherche par le contenu travaille encore. */
const ATTENTE_VISIBLE_MS = 350;
/** Ce qui tient sous la barre dans la fenêtre de l'île (640 px de haut). */
const NOMS_MAX = 4;
const CONTENU_MAX = 5;
/** La souris bouge de ça, bouton enfoncé, sur un résultat : on le glisse. */
const SEUIL_GLISSER = 6;
/** « Chemin copié » reste affiché ce temps. */
const NOTE_MS = 1400;
/** Le résultat choisi depuis ce temps : son aperçu est préparé. */
const PREPARER_APERCU_MS = 180;
/** La surbrillance glisse d'un résultat à l'autre. */
const GLISSE = { type: "spring", stiffness: 900, damping: 60, mass: 0.5 } as const;

/** Hauteurs (pixels CSS), pour que l'île prenne exactement la taille de la barre. */
export const HAUTEUR_CHAMP = 58;
const HAUTEUR_LIGNE = 54;
const HAUTEUR_TITRE = 26;
const HAUTEUR_MESSAGE = 42;
/** 6 + 14 de marges intérieures (de l'air au-dessus du bord arrondi), et la bordure d'un pixel. */
const MARGES_LISTE = 21;

interface Groupe {
  titre: "Par le nom" | "Par le contenu" | "Les plus récents" | "Récents";
  lignes: Ligne[];
}

function hauteur(groupes: Groupe[], message: boolean): number {
  const lignes = groupes.reduce((n, g) => n + g.lignes.length, 0);
  if (lignes === 0 && !message) return HAUTEUR_CHAMP;
  return HAUTEUR_CHAMP + MARGES_LISTE + lignes * HAUTEUR_LIGNE + groupes.length * HAUTEUR_TITRE + (message ? HAUTEUR_MESSAGE : 0);
}

function Nom({ ligne, recherche, surligne }: { ligne: Ligne; recherche: string; surligne: boolean }) {
  if (!surligne) return <>{ligne.nom}</>;
  return (
    <>
      {surligner(ligne.nom, recherche).map((m, i) => (m.trouve ? <strong key={i}>{m.texte}</strong> : <span key={i}>{m.texte}</span>))}
    </>
  );
}

/** L'extrait, avec les mots trouvés en gras (le moteur les entoure de \u0001 et \u0002). */
function Extrait({ texte }: { texte: string }) {
  const morceaux = texte.replace(/\s+/g, " ").split(/(\u0001[^\u0002]*\u0002)/);
  return (
    <>
      {morceaux.map((m, i) =>
        m.startsWith("\u0001") ? <strong key={i}>{m.slice(1, -1)}</strong> : <span key={i}>{m}</span>,
      )}
    </>
  );
}

/**
 * Pourquoi ce résultat : l'extrait où ses mots ont été trouvés, ou où regarder dans le fichier
 * (« page 4 », « dit 01:23–01:53 »), ou ce qu'il évoque pour une photo trouvée par ce qu'elle montre ;
 * puis son dossier.
 */
function Pourquoi({ ligne, recherche, parContenu }: { ligne: Ligne; recherche: string; parContenu: boolean }) {
  if (!parContenu) return <>{dossierCourt(ligne.dossier)}</>;
  if (ligne.extrait) {
    return (
      <>
        {ligne.position && `${ligne.position} · `}
        <Extrait texte={ligne.extrait} />
      </>
    );
  }
  const dossier = dossierCourt(ligne.dossier, 48);
  if (ligne.position) return <>{`${ligne.position} · ${dossier}`}</>;
  const evoque = ligne.genre === "photo" || ligne.genre === "video" ? "ressemble à" : "en rapport avec";
  return <>{`${evoque} « ${recherche} » · ${dossier}`}</>;
}

/** Un petit bouton d'action sur le résultat choisi (le clic ne l'ouvre pas). */
function Action({ titre, onClick, children }: { titre: string; onClick(): void; children: React.ReactNode }) {
  return (
    <button
      className="barre-action"
      title={titre}
      aria-label={titre}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      {children}
    </button>
  );
}

/**
 * La barre : par le nom (à chaque touche) puis par le contenu (après une courte pause). Flèches pour
 * choisir, Entrée pour ouvrir, Ctrl+Entrée pour le dossier, Espace pour l'aperçu, Ctrl+C pour copier le
 * chemin, Ctrl+O pour « Ouvrir avec », Échap pour fermer l'aperçu, effacer, puis fermer.
 */
export function Barre({
  depart,
  apercuDepart,
  indexation,
  onHauteur,
  onReglages,
}: {
  depart: string;
  /** Mode de test : l'aperçu de ce résultat s'ouvre dès qu'il est là. */
  apercuDepart: number | null;
  indexation: EtatIndexation;
  onHauteur(h: number): void;
  onReglages(): void;
}) {
  const [texte, setTexte] = useState(depart);
  const [noms, setNoms] = useState<{ pour: string; lignes: Ligne[] }>({ pour: "", lignes: [] });
  const [contenu, setContenu] = useState<{ pour: string; lignes: Ligne[] }>({ pour: "", lignes: [] });
  const [filtres, setFiltres] = useState<{ pour: string } & Filtres>({ pour: "", etiquettes: [], reste: "" });
  const [indexVide, setIndexVide] = useState(false);
  const [lent, setLent] = useState(false);
  const [choisi, setChoisi] = useState(0);
  const [apercu, setApercu] = useState(false);
  const [note, setNote] = useState<{ cle: string; texte: string } | null>(null);
  const [recents, setRecents] = useState<Ligne[] | null>(null);
  const mouvementReduit = useReducedMotion() ?? false;
  const champ = useRef<HTMLInputElement>(null);
  const derniere = useRef("");
  // Les flèches ont servi depuis la dernière frappe : Espace ouvre alors l'aperçu au lieu d'écrire.
  const navigue = useRef(false);
  const glisse = useRef<{ x: number; y: number; ligne: Ligne; parti: boolean } | null>(null);
  const recherche = texte.trim();
  // Le texte cherché sans les filtres (« photos 2024 chat » → « chat »).
  const mots = filtres.pour === recherche ? filtres.reste : recherche;
  const seulementFiltres = filtres.pour === recherche && filtres.etiquettes.length > 0 && filtres.reste === "";

  useEffect(() => {
    oublierApercus();
    etatIndex()
      .then((e) => setIndexVide(e.fichiers === 0))
      .catch(ignorer);
    chercherRecents()
      .then(setRecents)
      .catch(() => setRecents([]));
  }, []);

  // Par le nom : tout de suite. Une réponse pour un texte déjà changé est ignorée.
  useEffect(() => {
    derniere.current = recherche;
    setChoisi(0);
    setApercu(false);
    if (!recherche) {
      setNoms({ pour: "", lignes: [] });
      setContenu({ pour: "", lignes: [] });
      setFiltres({ pour: "", etiquettes: [], reste: "" });
      return;
    }
    lireFiltres(recherche)
      .then((f) => derniere.current === recherche && setFiltres({ pour: recherche, ...f }))
      .catch(ignorer);
    chercherNoms(recherche)
      .then((lignes) => derniere.current === recherche && setNoms({ pour: recherche, lignes }))
      .catch(ignorer);
  }, [recherche]);

  // Par le contenu : après une pause dans la frappe ; les anciens résultats restent en attendant.
  useEffect(() => {
    if (!recherche) return;
    let vivant = true;
    const depart = setTimeout(() => {
      chercherContenu(recherche)
        .then((lignes) => vivant && setContenu({ pour: recherche, lignes }))
        .catch(() => vivant && setContenu({ pour: recherche, lignes: [] }));
    }, PAUSE_CONTENU_MS);
    const signe = setTimeout(() => vivant && setLent(true), ATTENTE_VISIBLE_MS);
    return () => {
      vivant = false;
      clearTimeout(depart);
      clearTimeout(signe);
      setLent(false);
    };
  }, [recherche]);

  const apercuFait = useRef(false);
  useEffect(() => {
    if (apercuDepart === null || apercuFait.current || !contenuAJour || liste.length <= apercuDepart) return;
    apercuFait.current = true;
    setChoisi(apercuDepart);
    setApercu(true);
  });

  useEffect(() => {
    if (!note) return;
    const t = setTimeout(() => setNote(null), NOTE_MS);
    return () => clearTimeout(t);
  }, [note]);

  const groupes = useMemo(() => {
    const parNom = noms.lignes.slice(0, NOMS_MAX);
    const dejaLa = new Set(parNom.map((l) => l.cle));
    const parContenu = contenu.lignes.filter((l) => !dejaLa.has(l.cle)).slice(0, CONTENU_MAX + (parNom.length === 0 ? NOMS_MAX : 0));
    const g: Groupe[] = [];
    // La barre vide : les fichiers ouverts récemment.
    if (!recherche && recents && recents.length > 0) g.push({ titre: "Récents", lignes: recents });
    if (recherche && parNom.length > 0) g.push({ titre: "Par le nom", lignes: parNom });
    if (recherche && parContenu.length > 0) g.push({ titre: seulementFiltres ? "Les plus récents" : "Par le contenu", lignes: parContenu });
    return g;
  }, [noms, contenu, recherche, seulementFiltres, recents]);
  const liste = useMemo(() => groupes.flatMap((g) => g.lignes), [groupes]);
  const contenuAJour = contenu.pour === recherche;
  const choisie = liste[choisi] as Ligne | undefined;
  const enApercu = apercu && choisie !== undefined;
  // Un message seulement quand la recherche par le contenu a vraiment répondu (pas de « rien » qui clignote).
  const message =
    recherche === "" && recents?.length === 0
      ? "Essaie « facture électricité », « photos 2024 » ou « vacances Espagne »"
      : recherche !== "" && contenuAJour && liste.length === 0
      ? "Aucun résultat"
      : recherche !== "" && contenuAJour && indexVide
        ? "L'index est vide : indexe un dossier pour chercher aussi dans le contenu des fichiers"
        : null;

  // L'aperçu du résultat choisi se prépare dès qu'on s'y arrête : Espace l'affiche aussitôt.
  useEffect(() => {
    if (!choisie || !contenuAJour) return;
    const t = setTimeout(() => preparerApercu(choisie), PREPARER_APERCU_MS);
    return () => clearTimeout(t);
  }, [choisie, contenuAJour]);

  useEffect(
    () => onHauteur(enApercu ? HAUTEUR_CHAMP + HAUTEUR_APERCU : hauteur(groupes, message !== null)),
    [groupes, message, onHauteur, enApercu],
  );

  // La fenêtre reçoit le clavier un instant après l'ouverture ; le curseur va à la fin du texte de départ.
  useEffect(() => {
    const focus = () => {
      const c = champ.current;
      if (!c) return;
      // Sans défilement : pendant que l'île descend, elle est encore petite, et le navigateur ferait
      // défiler son contenu pour montrer le champ (tout restait ensuite remonté de 15 px).
      c.focus({ preventScroll: true });
      c.setSelectionRange(c.value.length, c.value.length);
    };
    focus();
    const t = setTimeout(focus, 60);
    window.addEventListener("focus", focus);
    return () => {
      clearTimeout(t);
      window.removeEventListener("focus", focus);
    };
  }, []);

  const ouvrir = (ligne: Ligne | undefined, dossier: boolean) => {
    if (ligne) ouvrirFichier(ligne, dossier).catch(ignorer);
  };
  const copier = (ligne: Ligne | undefined) => {
    if (ligne)
      copierChemin(ligne)
        .then(() => setNote({ cle: ligne.cle, texte: "Chemin copié" }))
        .catch(ignorer);
  };
  const ouvrirAutrement = (ligne: Ligne | undefined) => {
    if (ligne) ouvrirAvec(ligne).catch(ignorer);
  };

  let rang = 0;
  return (
    <div className="barre">
      <div className="barre-champ">
        <IconeRecherche />
        <input
          ref={champ}
          className="barre-saisie"
          value={texte}
          placeholder="Chercher un fichier par son nom ou ce qu'il contient"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            navigue.current = false;
            setTexte(e.target.value);
          }}
          onKeyDown={(e) => {
            const c = champ.current;
            const rienDeSelectionne = !c || c.selectionStart === c.selectionEnd;
            if (e.key === "Escape") {
              e.preventDefault();
              if (apercu) setApercu(false);
              else if (texte) setTexte("");
              else fermerBarre(true).catch(ignorer);
            } else if (e.key === " " && choisie && (apercu || navigue.current || e.shiftKey)) {
              e.preventDefault();
              setApercu((a) => !a);
            } else if (e.key === "ArrowDown") {
              e.preventDefault();
              navigue.current = true;
              setChoisi((c) => Math.min(c + 1, liste.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              navigue.current = true;
              setChoisi((c) => Math.max(c - 1, 0));
            } else if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && enApercu) {
              // Les pages du PDF (l'aperçu écoute ces touches) : le curseur du champ ne bouge pas.
              e.preventDefault();
            } else if (e.key === "Enter") {
              e.preventDefault();
              ouvrir(choisie, e.ctrlKey);
            } else if (e.ctrlKey && (e.key === "c" || e.key === "C") && rienDeSelectionne && choisie) {
              e.preventDefault();
              copier(choisie);
            } else if (e.ctrlKey && (e.key === "o" || e.key === "O") && choisie) {
              e.preventDefault();
              ouvrirAutrement(choisie);
            }
          }}
        />
        {filtres.pour === recherche && filtres.etiquettes.length > 0 && (
          <span className="barre-filtres" title="Filtres compris dans la recherche">
            {filtres.etiquettes.map((f) => (
              <span key={f} className="barre-filtre">
                {f}
              </span>
            ))}
          </span>
        )}
        {lent && !contenuAJour && <span className="barre-attente" aria-label="Recherche dans le contenu" />}
        <Anneau etat={indexation} />
        <button className="barre-bouton" title="Réglages" aria-label="Réglages" onClick={onReglages}>
          <IconeReglages />
        </button>
      </div>
      {enApercu ? (
        <Apercu ligne={choisie} mots={mots} />
      ) : (
        (groupes.length > 0 || message) && (
          <div className={`barre-liste ${lent && !contenuAJour ? "barre-liste--ancienne" : ""}`} role="listbox">
            {groupes.map((g) => (
              <div key={g.titre} className="barre-groupe">
                <div className="barre-titre">{g.titre}</div>
                {g.lignes.map((ligne) => {
                  const i = rang++;
                  const actif = i === choisi;
                  const parContenu = g.titre !== "Par le nom";
                  return (
                    <div
                      key={ligne.cle}
                      role="option"
                      aria-selected={actif}
                      className="barre-ligne"
                      onPointerMove={(e) => {
                        if (i !== choisi) setChoisi(i);
                        // Bouton enfoncé et souris partie : on glisse le fichier vers une autre appli.
                        const g = glisse.current;
                        if (g && !g.parti && e.buttons === 1 && Math.hypot(e.clientX - g.x, e.clientY - g.y) > SEUIL_GLISSER) {
                          g.parti = true;
                          glisser(g.ligne).catch(ignorer);
                        }
                      }}
                      onPointerDown={(e) => {
                        if (e.button === 0) glisse.current = { x: e.clientX, y: e.clientY, ligne, parti: false };
                      }}
                      onClick={(e) => {
                        const g = glisse.current;
                        glisse.current = null;
                        if (!g?.parti) ouvrir(ligne, e.ctrlKey);
                      }}
                    >
                      {actif && (
                        <motion.span
                          layoutId="barre-surbrillance"
                          className="barre-surbrillance"
                          transition={mouvementReduit ? { duration: 0 } : GLISSE}
                        />
                      )}
                      <Miniature ligne={ligne} />
                      <span className="barre-texte">
                        <span className="barre-nom">
                          <Nom ligne={ligne} recherche={mots} surligne={!parContenu} />
                          {/* L'extrait prend la place du dossier : on garde où il est, sinon deux
                              fichiers du même nom se ressembleraient. */}
                          {parContenu && ligne.extrait && <span className="barre-lieu">{dossierCourt(ligne.dossier, 40)}</span>}
                        </span>
                        <span className="barre-pourquoi">
                          <Pourquoi ligne={ligne} recherche={mots} parContenu={parContenu} />
                        </span>
                      </span>
                      {actif &&
                        (note?.cle === ligne.cle ? (
                          <span className="barre-indice">{note.texte}</span>
                        ) : (
                          <span className="barre-actions">
                            <Action titre="Aperçu (Espace)" onClick={() => setApercu(true)}>
                              <IconeOeil />
                            </Action>
                            <Action titre="Montrer dans le dossier (Ctrl+Entrée)" onClick={() => ouvrir(ligne, true)}>
                              <IconeDossier />
                            </Action>
                            <Action titre="Copier le chemin (Ctrl+C)" onClick={() => copier(ligne)}>
                              <IconeCopier />
                            </Action>
                            <Action titre="Ouvrir avec… (Ctrl+O)" onClick={() => ouvrirAutrement(ligne)}>
                              <IconeOuvrirAvec />
                            </Action>
                          </span>
                        ))}
                    </div>
                  );
                })}
              </div>
            ))}
            {message && <div className="barre-message">{message}</div>}
          </div>
        )
      )}
    </div>
  );
}
