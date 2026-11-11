import { decrire, type EtatIndexation } from "../ipc/indexation";

const R = 6.5;
const TOUR = 2 * Math.PI * R;

/**
 * L'indexation, discrète dans la barre : un petit anneau qui se remplit pendant la lecture et la
 * transcription, deux traits en pause. Rien quand l'index est à jour. Le détail est au survol.
 */
export function Anneau({ etat }: { etat: EtatIndexation }) {
  if (etat.phase === "a_jour" || etat.phase === "attente") return null;
  const titre = decrire(etat);
  if (etat.phase === "pause") {
    return (
      <span className="anneau anneau--pause" title={titre}>
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6 5v6M10 5v6" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      </span>
    );
  }
  if (etat.phase === "erreur") return <span className="anneau anneau--erreur" title={titre} />;
  const part = etat.phase === "parcours" ? null : etat.total > 0 ? etat.faits / etat.total : 0;
  return (
    <span className={`anneau ${part === null ? "anneau--tourne" : ""}`} title={titre}>
      <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
        <circle cx="8" cy="8" r={R} className="anneau-fond" />
        <circle
          cx="8"
          cy="8"
          r={R}
          className="anneau-plein"
          strokeDasharray={TOUR}
          strokeDashoffset={part === null ? TOUR * 0.7 : TOUR * (1 - part)}
        />
      </svg>
    </span>
  );
}
