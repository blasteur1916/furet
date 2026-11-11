// Prépare latest.json, le fichier que lit la vérification des mises à jour, à côté de l'installateur
// construit par `pnpm tauri build` (avec TAURI_SIGNING_PRIVATE_KEY). Ne publie rien : on joint ensuite
// l'installateur, sa signature et latest.json à la version « v<version> » de la page des versions.
//   node scripts/latest.mjs "Notes de version"
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const conf = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
const version = conf.version;
const installateur = `Furet_${version}_x64-setup.exe`;
const dossier = join(import.meta.dirname, "..", "..", "target", "release", "bundle", "nsis");
const signature = readFileSync(join(dossier, `${installateur}.sig`), "utf8").trim();
const depot = conf.plugins.updater.endpoints[0].replace(/\/releases\/latest\/download\/latest\.json$/, "");

const latest = {
  version,
  notes: process.argv[2] ?? "",
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": { signature, url: `${depot}/releases/download/v${version}/${installateur}` },
  },
};
writeFileSync(join(dossier, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
console.log(`latest.json prêt pour la version ${version} : ${join(dossier, "latest.json")}`);
