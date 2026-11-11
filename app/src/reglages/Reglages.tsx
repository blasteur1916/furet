import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { installerMiseAJour, raccourciActuel, taille, useInstallation, useMiseAJour } from "../ipc/installation";
import { basculerPause, decrire, indexerMaintenant, type EtatIndexation } from "../ipc/indexation";
import { ajouterDossier, ecrireReglages, lireReglages, retirerDossier, type Reglages as R } from "../ipc/reglages";
import { IconeRetour } from "../barre/icones";
import { dossierCourt } from "../barre/texte";
import { tr } from "../i18n";
import "./reglages.css";

const ignorer = () => {};

export function Interrupteur({ actif, onChange, children }: { actif: boolean; onChange(v: boolean): void; children: ReactNode }) {
  return (
    <label className="regl-option">
      <span>{children}</span>
      <button
        role="switch"
        aria-checked={actif}
        className={`regl-interrupteur ${actif ? "regl-interrupteur--on" : ""}`}
        onClick={() => onChange(!actif)}
      >
        <span className="regl-interrupteur-rond" />
      </button>
    </label>
  );
}

/**
 * Les réglages, dans l'île : les dossiers indexés, l'état de l'indexation, les options.
 * Échap ou la flèche reviennent à la recherche.
 */
export function Reglages({
  indexation,
  onRetour,
  onHauteur,
}: {
  indexation: EtatIndexation;
  onRetour(): void;
  onHauteur(h: number): void;
}) {
  const [reglages, setReglages] = useState<R | null>(null);
  const [erreur, setErreur] = useState<string | null>(null);
  const [raccourci, setRaccourci] = useState("Alt+Espace");
  const [version, setVersion] = useState("");
  const installation = useInstallation();
  const miseAJour = useMiseAJour();
  const racine = useRef<HTMLDivElement>(null);

  useEffect(() => {
    lireReglages().then(setReglages).catch(ignorer);
    raccourciActuel().then(setRaccourci).catch(ignorer);
    getVersion().then(setVersion).catch(ignorer);
  }, []);

  // L'île prend la hauteur réelle de la vue, même quand la liste des dossiers change.
  useLayoutEffect(() => {
    const el = racine.current;
    if (!el) return;
    const mesurer = () => onHauteur(Math.ceil(el.getBoundingClientRect().height));
    mesurer();
    const obs = new ResizeObserver(mesurer);
    obs.observe(el);
    return () => obs.disconnect();
  }, [onHauteur]);

  useEffect(() => {
    const touche = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onRetour();
      }
    };
    window.addEventListener("keydown", touche);
    return () => window.removeEventListener("keydown", touche);
  }, [onRetour]);

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

  const enCours = indexation.phase === "parcours" || indexation.phase === "lecture" || indexation.phase === "visages" || indexation.phase === "transcription";
  const enPause = indexation.phase === "pause";

  return (
    <div className="regl" ref={racine}>
      <div className="regl-tete">
        <button
          className="regl-retour"
          onClick={onRetour}
          aria-label={tr("Retour à la recherche", "Back to search")}
          title={tr("Retour (Échap)", "Back (Esc)")}
        >
          <IconeRetour />
        </button>
        <span className="regl-titre">{tr("Réglages", "Settings")}</span>
      </div>

      <div className="regl-defile">
      {miseAJour && (
        <div className="regl-section">
          <div className="regl-etat">
            <span className="regl-pastille regl-pastille--a_jour" />
            <span className="regl-etat-texte">
              {tr("Furet {} est prêt à être installé.", "Furet {} is ready to install.").replace("{}", miseAJour)}
            </span>
            <button className="regl-lien" onClick={() => installerMiseAJour().catch(ignorer)}>
              {tr("Redémarrer pour mettre à jour", "Restart to update")}
            </button>
          </div>
        </div>
      )}
      <div className="regl-section">
        <div className="regl-etat">
          <span className={`regl-pastille regl-pastille--${indexation.phase}`} />
          <span className="regl-etat-texte">{decrire(indexation)}</span>
          {(enCours || enPause) && (
            <button className="regl-lien" onClick={() => basculerPause().catch(ignorer)}>
              {enPause && indexation.raison === "manuelle" ? tr("Reprendre", "Resume") : enPause ? "" : tr("Pause", "Pause")}
            </button>
          )}
          {!enCours && !enPause && (
            <button className="regl-lien" onClick={() => indexerMaintenant().catch(ignorer)}>
              {tr("Indexer maintenant", "Index now")}
            </button>
          )}
        </div>
        {/* Les modèles d'une option qu'on vient d'activer (transcription, visages) arrivent. */}
        {installation?.phase === "telechargement" && (
          <div className="regl-etat">
            <span className="regl-pastille regl-pastille--lecture" />
            <span className="regl-etat-texte">
              {tr("Téléchargement", "Downloading")} : {installation.nom}… {taille(installation.fait)} {tr("sur", "of")}{" "}
              {taille(installation.total)}
            </span>
          </div>
        )}
        {installation?.phase === "erreur" && <div className="regl-erreur">{installation.message}</div>}
      </div>

      <div className="regl-section">
        <div className="regl-sous-titre">{tr("Dossiers indexés", "Indexed folders")}</div>
        <div className="regl-dossiers">
          {reglages?.dossiers.map((d) => (
            <div key={d} className="regl-dossier" title={d}>
              <span className="regl-dossier-nom">{dossierCourt(d, 64)}</span>
              <button
                className="regl-retirer"
                aria-label={`${tr("Retirer", "Remove")} ${d}`}
                title={tr("Retirer ce dossier (ses fichiers sortent de l'index)", "Remove this folder (its files leave the index)")}
                onClick={() => retirerDossier(d).then(setReglages).catch(ignorer)}
              >
                ×
              </button>
            </div>
          ))}
          {reglages && reglages.dossiers.length === 0 && <div className="regl-vide">{tr("Aucun dossier : ajoute-en un.", "No folder yet: add one.")}</div>}
        </div>
        <button className="regl-ajouter" onClick={ajouter}>
          + {tr("Ajouter un dossier", "Add a folder")}
        </button>
        {erreur && <div className="regl-erreur">{erreur}</div>}
      </div>

      {reglages && (
        <div className="regl-section">
          <div className="regl-sous-titre">Options</div>
          <Interrupteur actif={reglages.ocr} onChange={(ocr) => changer({ ...reglages, ocr })}>
            {tr("Lire le texte des images et des scans (OCR)", "Read text in images and scans (OCR)")}
          </Interrupteur>
          <Interrupteur actif={reglages.transcription} onChange={(transcription) => changer({ ...reglages, transcription })}>
            {tr("Transcrire la parole des audios et des vidéos", "Transcribe speech in audio and videos")}
          </Interrupteur>
          <Interrupteur actif={reglages.visages} onChange={(visages) => changer({ ...reglages, visages })}>
            {tr("Reconnaître les visages (« elliot.png » fait trouver Elliot partout)", "Recognize faces (“elliot.png” finds Elliot everywhere)")}
          </Interrupteur>
          <Interrupteur actif={reglages.bibliotheques} onChange={(bibliotheques) => changer({ ...reglages, bibliotheques })}>
            {tr("Lire aussi les grandes bibliothèques de code (moteurs de jeu, SDK : très long)", "Also read large code libraries (game engines, SDKs: very slow)")}
          </Interrupteur>
          <Interrupteur actif={reglages.pause_batterie} onChange={(pause_batterie) => changer({ ...reglages, pause_batterie })}>
            {tr("Pause quand le PC est sur batterie", "Pause on battery")}
          </Interrupteur>
          <Interrupteur actif={reglages.pause_jeu} onChange={(pause_jeu) => changer({ ...reglages, pause_jeu })}>
            {tr("Pause pendant les jeux et le plein écran", "Pause during games and full screen")}
          </Interrupteur>
          <Interrupteur actif={reglages.demarrage} onChange={(demarrage) => changer({ ...reglages, demarrage })}>
            {tr("Lancer Furet avec Windows", "Start Furet with Windows")}
          </Interrupteur>
          <div className="regl-option">
            <span>{tr("Langue", "Language")}</span>
            <span className="regl-choix">
              {(
                [
                  ["auto", tr("Automatique", "Automatic")],
                  ["en", "English"],
                  ["fr", "Français"],
                ] as const
              ).map(([code, nom]) => (
                <button
                  key={code}
                  className={`regl-choix-bouton ${reglages.langue === code ? "regl-choix-bouton--on" : ""}`}
                  onClick={() => {
                    if (reglages.langue === code) return;
                    // Tous les textes changent : la page se recharge une fois le réglage enregistré.
                    setReglages({ ...reglages, langue: code });
                    ecrireReglages({ ...reglages, langue: code })
                      .then(() => window.location.reload())
                      .catch(ignorer);
                  }}
                >
                  {nom}
                </button>
              ))}
            </span>
          </div>
        </div>
      )}

      <div className="regl-section">
        <div className="regl-sous-titre">{tr("À propos", "About")}</div>
        <div className="regl-apropos">
          <p>
            <strong>Furet {version}</strong> ·{" "}
            {tr(
              "logiciel libre (licence GPL-3.0). Tout se passe sur ce PC : rien de ce que tu cherches ni de tes fichiers n'est envoyé sur internet.",
              "free software (GPL-3.0 license). Everything happens on this PC: nothing you search for and none of your files is sent to the internet.",
            )}
          </p>
          <p>
            {tr(
              "Fonctionne grâce à EmbeddingGemma 2 de Google (fourni selon les conditions d'utilisation de Gemma, ai.google.dev/gemma/terms), llama.cpp (MIT), Parakeet TDT v3 de NVIDIA (CC BY 4.0) avec sherpa-onnx (Apache 2.0), YuNet (MIT) et SFace (Apache 2.0) d'OpenCV, PDFium (BSD), FFmpeg (GPL), les villes de GeoNames (geonames.org, CC BY 4.0) et les noms de pays du CLDR d'Unicode.",
              "Built on Google's EmbeddingGemma 2 (provided under the Gemma Terms of Use, ai.google.dev/gemma/terms), llama.cpp (MIT), NVIDIA's Parakeet TDT v3 (CC BY 4.0) with sherpa-onnx (Apache 2.0), OpenCV's YuNet (MIT) and SFace (Apache 2.0), PDFium (BSD), FFmpeg (GPL), GeoNames cities (geonames.org, CC BY 4.0) and Unicode CLDR country names.",
            )}
          </p>
        </div>
      </div>
      </div>

      <div className="regl-pied">
        {raccourci} {tr("pour chercher · tout reste sur ce PC", "to search · everything stays on this PC")}
      </div>
    </div>
  );
}
