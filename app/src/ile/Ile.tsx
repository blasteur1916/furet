import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Barre, HAUTEUR_CHAMP } from "../barre/Barre";
import { fermerBarre, onBarre, setMemoireBasse, type Ouverture } from "../ipc/ile";
import { useIndexation } from "../ipc/indexation";
import { dialogueOuvert } from "../ipc/reglages";
import { Reglages } from "../reglages/Reglages";
import { Accueil } from "../accueil/Accueil";
import { FERMEE, LARGEUR_BARRE, ouverte, targetBox } from "./geometrie";
import { IleShell } from "./IleShell";
import { useIslandRegion } from "./region";
import { DURATIONS, REGION_HOLD_MS } from "./springs";
import "./ile.css";

/** Fermée depuis ce temps, l'île demande à WebView2 de garder sa mémoire basse. */
const MEMOIRE_BASSE_APRES_MS = 3000;
/** La vue est posée à sa taille maximale : elle ne se remet jamais en page pendant la déformation. */
const HAUTEUR_VUE = 620;
const ease = [0.33, 1, 0.68, 1] as const;

type Vue = "recherche" | "reglages" | "accueil";

/**
 * L'île : cachée au-dessus du bord de l'écran, elle descend avec la barre quand le raccourci est
 * pressé, et grandit avec les résultats ou les réglages. Ailleurs que sur elle, les clics traversent
 * la fenêtre ; un clic ailleurs la referme.
 */
export function Ile() {
  const [ouvert, setOuvert] = useState(false);
  const [ouverture, setOuverture] = useState<Ouverture>({ texte: null, test: false, reglages: false, apercu: null, accueil: false });
  // Chaque ouverture a sa barre neuve : rouvrir aussitôt après une fermeture (double appui sur le
  // raccourci) ne garde ni l'ancien texte ni une hauteur qui ne correspond plus à la liste.
  const [numero, setNumero] = useState(0);
  const [vue, setVue] = useState<Vue>("recherche");
  const [hauteur, setHauteur] = useState(HAUTEUR_CHAMP);
  const indexation = useIndexation();
  const mouvementReduit = useReducedMotion() ?? false;

  useEffect(() => {
    let arreter: (() => void) | undefined;
    let vivant = true;
    onBarre(
      (o) => {
        setHauteur(HAUTEUR_CHAMP);
        setOuverture(o);
        setVue(o.accueil ? "accueil" : o.reglages ? "reglages" : "recherche");
        setNumero((n) => n + 1);
        setOuvert(true);
      },
      () => setOuvert(false),
    )
      .then((stop) => (vivant ? (arreter = stop) : stop()))
      .catch(() => {});
    return () => {
      vivant = false;
      arreter?.();
    };
  }, []);

  // Un clic ailleurs ferme l'île : le clavier y est déjà parti. Pas pendant le sélecteur de dossier
  // (il prend le clavier un moment), ni en mode de test.
  useEffect(() => {
    if (!ouvert || ouverture.test) return;
    const quitter = () => {
      if (!dialogueOuvert) fermerBarre(false).catch(() => {});
    };
    window.addEventListener("blur", quitter);
    return () => window.removeEventListener("blur", quitter);
  }, [ouvert, ouverture.test]);

  useEffect(() => {
    if (ouvert) {
      setMemoireBasse(false);
      return;
    }
    const t = setTimeout(() => setMemoireBasse(true), MEMOIRE_BASSE_APRES_MS);
    return () => clearTimeout(t);
  }, [ouvert]);

  const surHauteur = useCallback((h: number) => setHauteur(Math.min(h, HAUTEUR_VUE)), []);
  const versReglages = useCallback(() => setVue("reglages"), []);
  const versRecherche = useCallback(() => {
    setVue("recherche");
    setNumero((n) => n + 1);
  }, []);
  const cible = ouvert ? ouverte(hauteur) : FERMEE;
  const zone = targetBox(cible, window.innerWidth);
  useIslandRegion(zone ? [zone] : [], REGION_HOLD_MS);

  return (
    <div className="ile-scene">
      <IleShell target={cible} initial={FERMEE} reducedMotion={mouvementReduit}>
        <AnimatePresence initial={false}>
          {ouvert && (
            <motion.div
              key={`${vue}-${numero}`}
              className="ile-vue"
              style={{ width: LARGEUR_BARRE, height: HAUTEUR_VUE, x: "-50%" }}
              initial={{ opacity: 0, y: 4, scale: 0.98 }}
              animate={{
                opacity: 1,
                y: 0,
                scale: 1,
                transition: { duration: DURATIONS.contentIn, delay: DURATIONS.contentInDelay, ease },
              }}
              exit={{ opacity: 0, transition: { duration: DURATIONS.contentOut } }}
            >
              {vue === "accueil" ? (
                <Accueil onFini={versRecherche} onHauteur={surHauteur} />
              ) : vue === "recherche" ? (
                <Barre
                  depart={ouverture.texte ?? ""}
                  apercuDepart={ouverture.apercu}
                  indexation={indexation}
                  onHauteur={surHauteur}
                  onReglages={versReglages}
                />
              ) : (
                <Reglages indexation={indexation} onRetour={versRecherche} onHauteur={surHauteur} />
              )}
            </motion.div>
          )}
        </AnimatePresence>
      </IleShell>
    </div>
  );
}
