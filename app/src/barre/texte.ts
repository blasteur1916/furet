/** Sans accents ni majuscules : « Éditeur » et « editeur » se valent. */
function sansAccent(c: string): string {
  return c.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

/**
 * Les morceaux d'un nom, avec ceux qui correspondent aux mots cherchés, pour les mettre en gras.
 * La comparaison ignore accents et majuscules ; l'affichage garde le nom tel quel.
 */
export function surligner(nom: string, recherche: string): { texte: string; trouve: boolean }[] {
  // Le nom sans accents, et pour chaque lettre obtenue, la lettre d'origine.
  let simple = "";
  const origine: number[] = [];
  Array.from(nom).forEach((c, i) => {
    for (const s of sansAccent(c)) {
      simple += s;
      origine.push(i);
    }
  });
  const lettres = Array.from(nom);
  const trouve = new Array<boolean>(lettres.length).fill(false);
  for (const mot of sansAccent(recherche).split(/[\s\-_.]+/).filter((m) => m.length > 0)) {
    let depuis = 0;
    for (;;) {
      const at = simple.indexOf(mot, depuis);
      if (at < 0) break;
      for (let k = at; k < at + mot.length; k++) trouve[origine[k]] = true;
      depuis = at + mot.length;
    }
  }
  const morceaux: { texte: string; trouve: boolean }[] = [];
  lettres.forEach((c, i) => {
    const dernier = morceaux[morceaux.length - 1];
    if (dernier && dernier.trouve === trouve[i]) dernier.texte += c;
    else morceaux.push({ texte: c, trouve: trouve[i] });
  });
  return morceaux;
}

/** « D:\Photos\2024\Vacances\Bretagne » → « D:\…\Vacances\Bretagne » : la fin en dit le plus. */
export function dossierCourt(dossier: string, max = 60): string {
  if (dossier.length <= max) return dossier;
  const racine = dossier.slice(0, 3);
  return `${racine}…${dossier.slice(dossier.length - (max - racine.length - 1))}`;
}
