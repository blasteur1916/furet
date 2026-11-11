import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { lireApercu, type Apercu as A, type BlocTexte } from "../ipc/apercu";
import type { Ligne } from "../ipc/recherche";
import { Miniature, Picto } from "./icones";
import { surligner } from "./texte";
import "./apercu.css";

/** Ce qui tient sous la barre (l'île fait au plus 620 px de haut). */
export const HAUTEUR_APERCU = 562;
const PETITS_MOTS = new Set(["les", "des", "une", "aux", "pour", "par", "sur", "dans", "avec", "qui", "que", "est", "the", "and", "for", "with"]);

/** Les mots cherchés ressortent dans le texte (ceux d'une lettre, trop courants, non). */
function Surligne({ texte, mots }: { texte: string; mots: string }) {
  if (!mots) return <>{texte}</>;
  return (
    <>
      {surligner(texte, mots).map((m, i) => (m.trouve ? <mark key={i}>{m.texte}</mark> : <span key={i}>{m.texte}</span>))}
    </>
  );
}

function Texte({ blocs, cible, lignes, mots, marque }: { blocs: BlocTexte[]; cible: number; lignes: boolean; mots: string; marque: boolean }) {
  const zone = useRef<HTMLDivElement>(null);
  const vise = useRef<HTMLDivElement>(null);
  // Le passage trouvé en haut de la zone, avec un peu de contexte au-dessus.
  useLayoutEffect(() => {
    if (zone.current && vise.current) zone.current.scrollTop = Math.max(0, vise.current.offsetTop - 40);
  }, [blocs, cible]);
  return (
    <div className="apercu-corps apercu-corps--defile apercu-texte" ref={zone}>
      {blocs.map((b, i) => (
        <div key={i} ref={i === cible ? vise : undefined} className={i === cible && marque ? "apercu-bloc apercu-bloc--cible" : "apercu-bloc"}>
          {b.titre && <div className="apercu-bloc-titre">{b.titre}</div>}
          {lignes
            ? b.texte
                .replace(/\n$/, "")
                .split("\n")
                .map((l, k) => (
                  <div key={k} className="apercu-ligne">
                    <span className="apercu-num">{b.numero + k}</span>
                    <span>
                      <Surligne texte={l} mots={mots} />
                    </span>
                  </div>
                ))
            : <div className="apercu-para"><Surligne texte={b.texte.trim()} mots={mots} /></div>}
        </div>
      ))}
    </div>
  );
}

/** Une vidéo ou un son, lu à partir du moment trouvé. S'il ne se lit pas ici, sa grande miniature. */
function Lecteur({ adresse, debut, affiche, video }: { adresse: string; debut: number; affiche: string; video: boolean }) {
  const [illisible, setIllisible] = useState(false);
  const aller = (e: React.SyntheticEvent<HTMLMediaElement>) => {
    if (debut > 0) e.currentTarget.currentTime = debut;
  };
  if (illisible) {
    return (
      <div className="apercu-corps">
        <img className="apercu-image" src={affiche} alt="" draggable={false} />
        <div className="apercu-bandeau">Ne se lit pas ici : Entrée pour l'ouvrir</div>
      </div>
    );
  }
  return video ? (
    <div className="apercu-corps apercu-corps--noir">
      <video className="apercu-video" src={adresse} poster={affiche} controls autoPlay onLoadedMetadata={aller} onError={() => setIllisible(true)} />
    </div>
  ) : (
    <div className="apercu-corps apercu-son">
      <img className="apercu-pochette" src={affiche} alt="" draggable={false} />
      <audio className="apercu-audio" src={adresse} controls autoPlay onLoadedMetadata={aller} onError={() => setIllisible(true)} />
    </div>
  );
}

/** Une page de PDF ; ← et → changent de page. */
function Pdf({ adresse, page: depart, pages, onPage }: { adresse: string; page: number; pages: number; onPage(p: number): void }) {
  const [page, setPage] = useState(depart);
  const zone = useRef<HTMLDivElement>(null);
  useEffect(() => onPage(page), [page, onPage]);
  useEffect(() => {
    const touche = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") setPage((p) => Math.max(1, p - 1));
      else if (e.key === "ArrowRight") setPage((p) => Math.min(pages, p + 1));
    };
    window.addEventListener("keydown", touche);
    return () => window.removeEventListener("keydown", touche);
  }, [pages]);
  useLayoutEffect(() => {
    if (zone.current) zone.current.scrollTop = 0;
  }, [page]);
  return (
    <div className="apercu-corps apercu-corps--defile apercu-pdf" ref={zone}>
      <img
        className="apercu-page"
        src={`${adresse}${page}`}
        alt={`Page ${page}`}
        draggable={false}
        onLoad={() => zone.current && (zone.current.scrollTop = 0)}
      />
    </div>
  );
}

function Dossier({ entrees, total }: { entrees: { nom: string; dossier: boolean }[]; total: number }) {
  return (
    <div className="apercu-corps apercu-corps--defile apercu-dossier">
      {entrees.length === 0 && <div className="apercu-vide">Dossier vide</div>}
      {entrees.map((e) => (
        <div key={e.nom} className={`apercu-entree ${e.dossier ? "apercu-entree--dossier" : ""}`}>
          <Picto genre={e.dossier ? "dossier" : "fichier"} />
          <span>{e.nom}</span>
        </div>
      ))}
      {total > entrees.length && <div className="apercu-vide">et {total - entrees.length} autres…</div>}
    </div>
  );
}

/**
 * L'aperçu du résultat choisi, à la place de la liste : Espace ou Échap le referme, ↑ et ↓ passent au
 * résultat voisin, Entrée l'ouvre.
 */
export function Apercu({ ligne, mots }: { ligne: Ligne; mots: string }) {
  const [vu, setVu] = useState<{ cle: string; apercu: A | null }>({ cle: "", apercu: null });
  const [page, setPage] = useState(0);
  useEffect(() => {
    let vivant = true;
    lireApercu(ligne)
      .then((a) => vivant && setVu({ cle: ligne.cle, apercu: a }))
      .catch(() => vivant && setVu({ cle: ligne.cle, apercu: null }));
    return () => {
      vivant = false;
    };
  }, [ligne]);
  const a = vu.cle === ligne.cle ? vu.apercu : null;
  const c = a?.contenu;
  // Les petits mots (« au », « de », « les ») ne sont pas surlignés : ils sont partout.
  const motsUtiles = mots
    .split(/[\s'’]+/)
    .filter((m) => m.length >= 3 && !PETITS_MOTS.has(m.toLowerCase()))
    .join(" ");

  return (
    <div className="apercu">
      <div className="apercu-tete">
        <Miniature ligne={ligne} />
        <span className="apercu-titre">
          <span className="apercu-nom">{ligne.nom}</span>
          <span className="apercu-infos">
            {/* La position, sauf « modifié le… » ou « pris à… » : les infos le disent déjà. */}
            {[c?.type === "pdf" ? `page ${page || c.page} sur ${c.pages}` : /^(modifié|pris)/.test(ligne.position) ? "" : ligne.position, a?.infos]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </span>
        <span className="apercu-aide">
          {c?.type === "pdf" && c.pages > 1 && <kbd>← →</kbd>}
          <kbd>Espace</kbd> fermer
        </span>
      </div>
      {!c ? (
        <div className="apercu-corps" />
      ) : c.type === "image" ? (
        <div className="apercu-corps">
          <img className="apercu-image" src={c.adresse} alt="" draggable={false} />
        </div>
      ) : c.type === "video" || c.type === "audio" ? (
        <Lecteur adresse={c.adresse} debut={c.debut} affiche={c.affiche} video={c.type === "video"} />
      ) : c.type === "pdf" ? (
        <Pdf adresse={c.adresse} page={c.page} pages={c.pages} onPage={setPage} />
      ) : c.type === "texte" ? (
        <Texte blocs={c.blocs} cible={c.cible} lignes={c.lignes} mots={motsUtiles} marque={ligne.repere !== null} />
      ) : c.type === "dossier" ? (
        <Dossier entrees={c.entrees} total={c.total} />
      ) : (
        <div className="apercu-corps">
          <img className="apercu-image apercu-image--icone" src={c.adresse} alt="" draggable={false} />
        </div>
      )}
    </div>
  );
}
