import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { installer, raccourciActuel, taille, useInstallation } from "../ipc/installation";
import { ajouterDossier, ecrireReglages, lireReglages, retirerDossier, type Reglages as R } from "../ipc/reglages";
import { dossierCourt } from "../barre/texte";
import { Interrupteur } from "../reglages/Reglages";
import "../reglages/reglages.css";
import "./accueil.css";

const ignorer = () => {};

/**
 * Le premier lancement, dans l'île : ce que fait Furet, les dossiers à explorer, les options, puis le
 * téléchargement de ses outils et de son modèle (une fois pour toutes). Il continue si l'île se ferme.
 */
export function Accueil({ onFini, onHauteur }: { onFini(): void; onHauteur(h: number): void }) {
  const installation = useInstallation();
  const [reglages, setReglages] = useState<R | null>(null);
  const [raccourci, setRaccourci] = useState("Alt+Espace");
  const [erreur, setErreur] = useState<string | null>(null);
  // Le téléchargement a été lancé d'ici : à la fin, on dit « c'est prêt » au lieu de repartir sur la barre.
  const [lance, setLance] = useState(false);
  const racine = useRef<HTMLDivElement>(null);

  useEffect(() => {
    lireReglages().then(setReglages).catch(ignorer);
    raccourciActuel().then(setRaccourci).catch(ignorer);
  }, []);

  useLayoutEffect(() => {
    const el = racine.current;
    if (!el) return;
    const mesurer = () => onHauteur(Math.ceil(el.getBoundingClientRect().height));
    mesurer();
    const obs = new ResizeObserver(mesurer);
    obs.observe(el);
    return () => obs.disconnect();
  }, [onHauteur]);

  const changer = (r: R) => {
    setReglages(r);
    ecrireReglages(r).then(setReglages).catch(ignorer);
  };
  const ajouter = () => {
    setErreur(null);
    ajouterDossier()
      .then(setReglages)
      .catch((e: unknown) => setErreur(String(e)));
  };
  const commencer = () => {
    setLance(true);
    installer().catch(ignorer);
  };

  const phase = installation?.phase;
  const aTelecharger =
    installation?.phase === "a_installer" && reglages
      ? installation.essentiel + (reglages.transcription ? installation.transcription : 0) + (reglages.visages ? installation.visages : 0)
      : 0;
  const pret = phase === "pret";

  return (
    <div className="regl accueil" ref={racine}>
      <div className="accueil-tete">
        <span className="accueil-titre">Bienvenue dans Furet</span>
        <span className="accueil-texte">
          Retrouve n'importe quel fichier par son nom ou par ce qu'il contient : photos, PDF, documents, vidéos,
          sons… Tout reste sur ce PC : rien n'est envoyé sur internet.
        </span>
      </div>

      {!pret && phase !== "telechargement" && reglages && (
        <>
          <div className="regl-section">
            <div className="regl-sous-titre">Dossiers à explorer</div>
            <div className="regl-dossiers">
              {reglages.dossiers.map((d) => (
                <div key={d} className="regl-dossier" title={d}>
                  <span className="regl-dossier-nom">{dossierCourt(d, 64)}</span>
                  <button
                    className="regl-retirer"
                    aria-label={`Retirer ${d}`}
                    onClick={() => retirerDossier(d).then(setReglages).catch(ignorer)}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
            <button className="regl-ajouter" onClick={ajouter}>
              + Ajouter un dossier
            </button>
            {erreur && <div className="regl-erreur">{erreur}</div>}
          </div>

          <div className="regl-section">
            <div className="regl-sous-titre">Options</div>
            <Interrupteur actif={reglages.transcription} onChange={(transcription) => changer({ ...reglages, transcription })}>
              Transcrire la parole des sons et des vidéos
              {installation?.phase === "a_installer" && installation.transcription > 0 && (
                <span className="accueil-poids"> + {taille(installation.transcription)}</span>
              )}
            </Interrupteur>
            <Interrupteur actif={reglages.visages} onChange={(visages) => changer({ ...reglages, visages })}>
              Reconnaître les visages
              {installation?.phase === "a_installer" && installation.visages > 0 && (
                <span className="accueil-poids"> + {taille(installation.visages)}</span>
              )}
            </Interrupteur>
            <Interrupteur actif={reglages.demarrage} onChange={(demarrage) => changer({ ...reglages, demarrage })}>
              Lancer Furet avec Windows
            </Interrupteur>
          </div>
        </>
      )}

      <div className="regl-section accueil-action">
        {phase === "telechargement" && installation?.phase === "telechargement" ? (
          <>
            <div className="accueil-barre">
              <div className="accueil-barre-plein" style={{ width: `${(100 * installation.fait) / Math.max(1, installation.total)}%` }} />
            </div>
            <div className="accueil-ligne">
              <span>{installation.nom}…</span>
              <span>
                {taille(installation.fait)} sur {taille(installation.total)}
              </span>
            </div>
            <div className="accueil-note">Tu peux fermer cette fenêtre : le téléchargement continue.</div>
          </>
        ) : pret ? (
          <>
            <div className="accueil-pret">
              {lance ? "C'est prêt !" : "Tout est installé."} Furet lit tes fichiers en arrière-plan ; tu peux déjà
              chercher.
            </div>
            <button className="accueil-bouton" onClick={onFini}>
              Commencer à chercher
            </button>
          </>
        ) : (
          <>
            {phase === "erreur" && installation?.phase === "erreur" && <div className="regl-erreur">{installation.message}</div>}
            <button className="accueil-bouton" onClick={commencer} disabled={!reglages}>
              {phase === "erreur" ? "Réessayer" : `Télécharger et commencer · ${taille(aTelecharger)}`}
            </button>
            <div className="accueil-note">
              Une seule fois : le moteur de recherche et son modèle. Ensuite, ouvre Furet à tout moment avec{" "}
              <strong>{raccourci}</strong>.
            </div>
          </>
        )}
      </div>
    </div>
  );
}
