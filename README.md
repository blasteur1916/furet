# Furet

**Retrouve n'importe quel fichier par son nom ou par ce qu'il contient, en une seule barre, entièrement sur ton PC.**

Appuie sur **Alt+Espace**, tape ce dont tu te souviens : « facture d'électricité d'octobre », « photo d'un chat sur un canapé », « la réunion où on parlait du budget », « photos Espagne 2019 ». Furet cherche dans le nom des fichiers, dans le texte des documents, dans ce que montrent les photos et les vidéos, et dans ce qui est dit dans les sons. Rien n'est envoyé sur internet.

> Furet est la suite de Quiera, du même auteur. Logiciel libre sous licence GPL-3.0.

---

## Ce que Furet sait faire

### Chercher par le sens, pas seulement par les mots
- **Photos** : ce qu'elles montrent (« coucher de soleil sur la mer ») et le texte qu'elles contiennent (OCR : captures d'écran, tickets, documents photographiés).
- **PDF, Word, PowerPoint, OpenDocument, Excel, CSV** : leur texte, y compris les PDF scannés (OCR).
- **Vidéos** : leurs images et leur son ; la parole est transcrite (« la vidéo où il explique le montage »).
- **Sons et enregistrements** : ce qui y est dit, et leur ambiance.
- **Textes et code** : notes, journaux, scripts, projets.
- **Archives .zip** : les photos, PDF, documents et notes qu'elles contiennent, sans les ouvrir.

Les **mots exacts** comptent aussi : un numéro de facture ou un nom propre est retrouvé même s'il n'a « aucun sens », avec le passage où il apparaît.

### Chercher par le nom, partout
Tous les fichiers et dossiers de tous les disques sont trouvables par leur nom en quelques millisecondes (près de 2 millions de noms), et la liste suit les changements en direct. Dans les dossiers indexés, une faute de frappe est tolérée.

### Personnes et lieux
- **Visages** : nomme une photo `elliot.jpg`, et chercher « elliot » retrouve aussi les autres photos où Elliot apparaît, même si elles s'appellent `IMG_2045.jpg`.
- **Lieux** : « vacances Espagne » ou « photos Barcelone » mettent en tête les photos prises là (d'après leur GPS ; aucun service en ligne).
- **Dates de prise de vue** : « photos 2019 » utilise la date où la photo a été prise, pas celle de sa copie.

### Filtres tapés dans la barre
| Tu tapes | Furet comprend |
|---|---|
| `pdf facture` | les PDF qui parlent de factures |
| `photos 2024` | les photos de 2024 |
| `vidéos > 1 Go` | les vidéos de plus d'1 Go |
| `devis moins de 2 Mo` | les fichiers de moins de 2 Mo qui parlent de devis |
| `modifié la semaine dernière` | les fichiers modifiés la semaine dernière |
| `contrat avant 2020`, `notes d'hier`, `photos mars 2023` | … |

Les filtres compris s'affichent en pastilles à droite de la barre.

### Aperçu et actions
- **Espace** (après les flèches) ou **Maj+Espace** : aperçu sans ouvrir — la photo, la page du PDF (← → pour tourner les pages), la vidéo ou le son lu **au moment trouvé**, le texte autour du passage, le contenu d'un dossier ou d'une archive.
- **Entrée** : ouvrir · **Ctrl+Entrée** : montrer dans le dossier · **Ctrl+C** : copier le chemin · **Ctrl+O** : ouvrir avec…
- **Glisser** un résultat vers une autre appli ou un dossier (le fichier est copié, jamais déplacé).
- La barre vide propose les **fichiers ouverts récemment** ; ceux que tu ouvres souvent remontent dans les résultats.

### Discret
- L'indexation tourne en arrière-plan, en priorité basse, et **se met en pause sur batterie, pendant un jeu ou une appli en plein écran** (la carte graphique est alors libérée).
- Les recherches se font sur le processeur (environ 30 ms), la carte graphique reste au repos.
- Le code des grandes bibliothèques (moteurs de jeu, SDK) et les archives de logiciels sont cherchés par leur nom seulement : l'index reste léger et les résultats restent les tiens.

---

## Installer

1. Télécharge `Furet_1.0.0_x64-setup.exe` depuis la page des [versions](https://github.com/blasteur1916/furet/releases/latest).
2. Lance-le. L'installateur n'est pas encore signé : Windows peut afficher « Windows a protégé votre PC ». Clique sur **Informations complémentaires**, puis **Exécuter quand même**.
3. Au premier lancement, Furet s'ouvre sur son accueil : choisis les dossiers à explorer et les options, puis **Télécharger et commencer**. Il télécharge une seule fois son moteur et ses modèles (environ 1,1 Go, 1,6 Go avec la transcription et les visages), chacun vérifié. Un téléchargement coupé reprend où il en était.
4. Ensuite, **Alt+Espace** ouvre la barre depuis n'importe où. Si Alt+Espace est déjà pris par un autre programme, Furet prend Ctrl+Alt+Espace (le raccourci actuel est indiqué dans les réglages).

Les mises à jour se téléchargent toutes seules ; un bouton **« Redémarrer pour mettre à jour »** apparaît dans les réglages.

### Configuration
- Windows 10 ou 11, 64 bits.
- Environ 2 Go d'espace pour le moteur et les modèles, plus l'index (quelques centaines de Mo).
- Une carte graphique compatible Vulkan accélère beaucoup l'indexation (NVIDIA, AMD, Intel Arc) ; sans elle, Furet indexe sur le processeur, plus lentement.
- La lecture du texte des images utilise l'OCR de Windows, dans les langues installées sur le PC.

---

## Vie privée

- **Tout se passe sur ton PC.** Les fichiers, ce que tu cherches et l'index ne quittent jamais la machine.
- Furet ne se connecte à internet que pour télécharger son moteur et ses modèles au premier lancement, et pour vérifier les mises à jour.
- Les journaux de l'appli ne contiennent ni le contenu des fichiers ni ce que tu cherches.
- Tout est rangé dans `%LOCALAPPDATA%\app.furet` (index, modèles) et `%APPDATA%\app.furet` (réglages, historique). Retirer un dossier des réglages retire ses fichiers de l'index.

---

## Réglages

Roue dentée à droite de la barre :
- dossiers à indexer ;
- lire le texte des images et des scans (OCR), transcrire la parole, reconnaître les visages ;
- lire aussi le code des grandes bibliothèques (moteurs de jeu, SDK — très long) ;
- pause sur batterie, pendant les jeux et le plein écran, ou à la demande ;
- lancer Furet avec Windows.

---

## Comment ça marche

| Pour | Furet utilise |
|---|---|
| Le sens des textes, images et sons | [EmbeddingGemma 2](https://huggingface.co/google/embeddinggemma-2) de Google, avec [llama.cpp](https://github.com/ggml-org/llama.cpp) (Vulkan ou processeur) |
| La transcription de la parole | [Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) de NVIDIA, avec [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) |
| Le texte des images et des scans | l'OCR de Windows |
| Les PDF | [PDFium](https://pdfium.googlesource.com/pdfium/) |
| Les sons et les vidéos | [FFmpeg](https://ffmpeg.org) |
| Les visages | YuNet et SFace d'[OpenCV](https://github.com/opencv/opencv_zoo), avec [tract](https://github.com/sonos/tract) |
| Les lieux | les villes de [GeoNames](https://www.geonames.org) et les noms de pays du [CLDR](https://cldr.unicode.org) |
| L'index | SQLite (avec sqlite-vec et FTS5) et un index HNSW ([usearch](https://github.com/unum-cloud/usearch)) |
| L'appli | [Tauri 2](https://tauri.app), Rust et React |

Les détails, les mesures et les choix sont dans [RECHERCHE.md](RECHERCHE.md).

### Le code
```
moteur/   la librairie : indexation, recherche, OCR, transcription, visages, lieux, filtres…
proto/    un outil en ligne de commande (index, cherche, stats)
app/      l'appli : interface React dans app/src, Rust dans app/src-tauri
```

### Compiler

Il faut [Rust](https://rustup.rs) (1.88 ou plus récent), [Node.js](https://nodejs.org) et [pnpm](https://pnpm.io).

```powershell
cd app
pnpm install
pnpm tauri build --no-bundle   # → target\release\furet.exe
pnpm tauri build               # → aussi l'installateur, dans target\release\bundle\nsis
```

Au premier lancement, l'appli compilée télécharge son moteur et ses modèles comme la version installée. La variable `FURET_RACINE` permet de lui donner un autre dossier.

L'outil en ligne de commande :
```powershell
cargo build --release -p furet-proto
target\release\furet-proto.exe --racine <dossier de Furet> index C:\Users\moi\Documents
target\release\furet-proto.exe --racine <dossier de Furet> cherche "facture d'électricité"
```

---

## Pas encore

- **Linux** : prévu. Le moteur est écrit pour être portable, mais l'OCR, les miniatures, le raccourci et la forme de la fenêtre sont encore propres à Windows.
- **Installateur signé** : pour supprimer l'avertissement de Windows à l'installation.

---

## Licence

Furet est un logiciel libre sous licence [GPL-3.0](LICENSE).

Il télécharge et utilise :
- EmbeddingGemma 2, fourni selon les [conditions d'utilisation de Gemma](https://ai.google.dev/gemma/terms) ;
- llama.cpp (MIT) ;
- Parakeet TDT v3 (CC BY 4.0) et sherpa-onnx (Apache 2.0) ;
- YuNet (MIT) et SFace (Apache 2.0) ;
- PDFium (BSD) ;
- FFmpeg (GPL) ;
- les données de GeoNames (CC BY 4.0) et du CLDR d'Unicode (licence Unicode).
