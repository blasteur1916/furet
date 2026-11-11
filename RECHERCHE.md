# Furet (nom provisoire) — recherche avant de commencer

Une appli Windows qui retrouve un fichier **par ce qu'il contient** : une photo par ce qu'elle montre, un PDF par une phrase, un enregistrement par ce qui s'y dit, une vidéo au moment exact d'une scène. Et, dans la même barre, un fichier **par son nom**, instantanément, comme Everything.

Tout tourne sur le PC : pas de compte, rien n'est envoyé.

Relevé le 8 novembre 2026. Les liens sont en bas de page.

---

## 1. Le principe

Deux index qui répondent ensemble à chaque recherche :

1. **Par le nom** : la liste de tous les fichiers des disques, prête en quelques secondes, tenue à jour en continu.
2. **Par le contenu** : chaque photo, page, passage de texte, moment de vidéo et tranche de son devient un **vecteur** (une suite de nombres qui résume son sens). La recherche transforme ta phrase en vecteur et cherche les plus proches. Un index de **mots exacts** complète, car aucun vecteur ne retient un numéro de facture.

Un seul modèle qui place texte, images, son et vidéo **dans le même espace** permet de classer une image contre une page de PDF avec une seule requête.

---

## 2. Le modèle : EmbeddingGemma 2 (Google DeepMind)

Le meilleur choix aujourd'hui pour une appli locale qui couvre **tous** les types de fichiers.

- **Texte, code, images, vidéo et son** dans un seul espace de 768 dimensions (réductible à 512, 256 ou 128 pour gagner de la place).
- **Léger** : environ 740 M de paramètres au total ; 865 Mo à télécharger une fois (310 Mo pour le texte, 555 Mo pour les encodeurs image et son).
- **Licence Apache 2.0** : utilisable librement, y compris commercialement.
- **Scores** (fiche officielle) : MTEB multilingue 61,4 · code 78,7 · images MIEB lite 65,7 · son MSEB 69,5 · vidéo MMEB v2 50,7.
- **Tourne dans llama.cpp** (prise en charge ajoutée le 6 octobre 2026), sur la carte graphique via **Vulkan** (NVIDIA, AMD, Intel) ou sur le processeur.

**Vérifié le 8 novembre 2026** : l'image et le son passent par un fichier « projecteur » séparé (`mmproj`). Le ticket llama.cpp #30082 qui disait ce chemin cassé a été fermé par son auteur (mauvaise forme de requête). Image, son et vidéo marchent avec `llama-server` sur `/v1/embeddings`, au format `{"input": [{"content": [{"type": "image_url" | "input_audio", …}]}]}`. Voir la partie 10.

### Les alternatives

| Modèle | Ce qu'il couvre | Pour | Contre |
|---|---|---|---|
| **Qwen3-VL-Embedding 2B** | texte, images, vidéo | les meilleurs scores image et vidéo (MMEB-V2), Apache 2.0, GGUF disponible | **pas de son**, 2 milliards de paramètres (plus lourd), dimension 2048 |
| **jina-embeddings-v5-omni** | texte, images, son, vidéo | très complet | licence **non commerciale** par défaut |
| **SigLIP 2** | texte et images | petit, rapide | pas de son ni de vidéo, contexte texte court |

Recommandation : **EmbeddingGemma 2 partout**, et garder Qwen3-VL en option si la qualité sur les photos ne suffit pas.

---

## 3. Type de fichier par type de fichier

| Type | Lire le fichier | Comprendre le contenu | Remarques |
|---|---|---|---|
| **Noms de tous les fichiers** | Table **MFT** du NTFS et **journal USN** (crate Rust `ntfs-reader`) | — | Tous les noms en quelques secondes, mises à jour en direct. Demande des droits administrateur : un petit service installé une fois, comme Everything. Sans droits : parcours classique des dossiers (ce que fait le lanceur de Quiera). |
| **PDF** | `pdfium-render` (moteur PDF de Chrome) | découpage en passages → vecteurs texte | Ouvrir le PDF **à la bonne page**. Les PDF scannés passent par l'OCR. |
| **Word, PowerPoint** | Office XML (zip + `quick-xml`), ou `kreuzberg` | vecteurs texte | Les vieux `.doc` sont plus difficiles. |
| **Excel, ODS** | `calamine` (standard en Rust) | vecteurs texte par feuille ou bloc | |
| **Texte, Markdown, code** | lecture directe ; code découpé par fonction (`tree-sitter`) | vecteurs texte et code | Ouvrir le code **à la ligne**, dans l'éditeur choisi. |
| **Photos** (JPEG, PNG, HEIC, RAW…) | **WIC**, le décodeur d'images de Windows (lit HEIC et RAW si les extensions sont installées) | encodeur image | Miniatures via le cache de miniatures de Windows. |
| **Captures d'écran, scans** | WIC | encodeur image **et OCR** | **OCR intégré à Windows** (`Windows.Media.Ocr`) : gratuit, rapide, plus de 25 langues. Option PaddleOCR (ONNX) plus tard pour les petits textes. **DigUp n'a pas l'OCR sur Windows.** |
| **Audio** (MP3, WAV, FLAC, M4A…) | `ffmpeg` (ou Media Foundation) | encodeur son par **tranches de 30 s** + **transcription** | La transcription donne les mots exacts et le moment. **Whisper large-v3-turbo** : 99 langues, licence MIT. **Parakeet TDT v3** : bien plus rapide, meilleur en français, 25 langues européennes, licence CC BY 4.0. |
| **Vidéo** | `ffmpeg` (décodage matériel), **détection des changements de plan** | une image par plan → encodeur image ; piste son → transcription | Ouvrir la vidéo **au moment exact**. Plus tard : glisser le passage dans Premiere ou DaVinci. |

---

## 4. Le moteur

- **Appli** : Tauri 2 + Rust + React, comme Quiera. On réutilise le lanceur, l'installateur, les mises à jour signées, le site et le pipeline de release.
- **Calcul** : **llama.cpp** avec Vulkan (toutes les cartes graphiques) et repli sur le processeur. ONNX Runtime avec Windows ML en alternative (DirectML est en maintenance chez Microsoft, les nouveautés passent par Windows ML).
- **Stockage** :
  - **SQLite** pour les fichiers, les passages et l'avancement ;
  - **FTS5** (dans SQLite) pour les mots exacts ;
  - les vecteurs : **sqlite-vec** pour démarrer (simple, mais recherche exhaustive : bien jusqu'à ~100 000 vecteurs), puis **usearch** ou **LanceDB** (index HNSW) quand on dépasse.
- **Indexation en arrière-plan** : priorité basse, pause sur batterie et en jeu, reprise après redémarrage.

---

## 5. La concurrence (vérifiée)

- **DigUp** (`ARahim3/DigUp`) : 6 300 étoiles en un mois, licence MIT, même modèle (EmbeddingGemma 2). Né sur Mac.
  - Son moteur est en **Swift** jusque sur Windows (rare et lourd à maintenir).
  - La version Windows est en **bêta**, et l'installateur n'est **pas signé**.
  - **Pas d'OCR sur Windows**.
  - Un ticket ouvert sur la **recherche par nom de fichier**.
- **Recherche sémantique de Windows** : réservée aux PC Copilot+ (NPU d'au moins 40 TOPS), et seulement dans les dossiers indexés.
- **Payants** : Fenn (Mac seulement), FileScope (Windows, sans vidéo ni audio), BeaconFind, PixaFind, LocalSpider (gratuit pendant la bêta).
- **Filect** envoie le contenu des fichiers à OpenAI.
- **Khoj** et **Coco AI** : plutôt des assistants de discussion et des lanceurs, pas des chercheurs de fichiers.

### Ce qui peut nous distinguer

1. **Nom et sens dans la même barre** : instantané par le nom dès l'installation (MFT), le contenu qui s'ajoute pendant l'indexation.
2. **Vraiment Windows** : OCR des captures, installateur signé, raccourci Alt+Espace, appli native Rust de bout en bout.
3. **Pour les monteurs** : retrouver un plan par sa description ou par ce qui s'y dit, puis le glisser dans Premiere ou DaVinci.

---

## 6. Risques

- **Le modèle pèse 865 Mo** et l'indexation d'un gros disque prend des heures : il faut afficher l'avancement et laisser choisir les dossiers.
- **Batterie et chaleur** sur portable : pause automatique sur batterie.
- **llama.cpp et l'image et le son** (ticket #30082) : validé, ça marche (partie 10).
- **Jeux et autres logiciels** : un jeu lancé (Overwatch) triple le temps de calcul sur la carte graphique. La pause en jeu est indispensable, et toute mesure doit se faire PC au repos.
- **DigUp avance vite** : se concentrer sur ce qu'il ne fait pas.

---

## 7. Le design : celui du lanceur de Quiera, agrandi

On garde **le langage visuel de la recherche de Quiera** : il plaît, il est déjà fait, et il distingue l'appli des fenêtres classiques. On l'adapte à ce qui est nouveau, les **aperçus**.

### Ce qu'on garde tel quel

- **L'île qui descend du haut de l'écran** : une forme noire collée au bord, avec les petites courbes qui la relient à l'écran (« flares »), qui se déforme en douceur à l'ouverture (ressorts Motion). Code : `src/island/geometry.ts`, `springs.ts`, `IslandShell.tsx`.
- **Les couleurs** (`src/styles/tokens.css`) :
  - fond noir pur `#000`, cartes `#121214`, survol `#1a1a1d` ;
  - texte blanc `#fff`, secondaire blanc à 62 %, discret blanc à 36 % ;
  - ligne sélectionnée blanc à 9 % ;
  - curseur de saisie bleu `#0a84ff` ;
  - rouge `#ff6b5e` pour les erreurs.
- **Les polices** : Segoe UI Variable Text (interface) et Display (titres), celles de Windows 11.
- **La barre de recherche** (`src/modules/launcher/launcher.css`) :
  - hauteur 58 px, texte 17 px en semi-gras, icône loupe à gauche ;
  - lignes de résultats de 40 px, coins « squircle » de 13 px ;
  - titre 13 à 14 px avec les lettres trouvées en gras, sous-titre 10,5 px ;
  - l'indication de touche (« Ouvrir ↵ ») à droite de la ligne sélectionnée.
- **Le clavier** : Alt+Espace (modifiable), ↑ ↓, Entrée pour ouvrir, Ctrl+Entrée pour le dossier, Échap pour fermer.

### Ce qu'on ajoute

- **Plus large quand il y a des aperçus** : 640 px au repos comme dans Quiera, jusqu'à environ 880 px avec le panneau d'aperçu.
- **Des lignes de 52 px avec une miniature** et une ligne « pourquoi » sous le titre, par exemple :
  - « page 4 · …le montant total de la **facture**… » ;
  - « 01:23 · “on parle du **sommeil**” » ;
  - « photo · ressemble à : chien sur la plage ».
- **Un panneau d'aperçu à droite** pour le résultat choisi :
  - photo en grand ;
  - page du PDF avec les mots surlignés ;
  - image de la vidéo au bon moment, avec la frise temporelle ;
  - extrait de la transcription audio ;
  - lignes du code.

  Espace l'agrandit, comme Coup d'œil sur Mac.
- **Des filtres en pastilles** sous la barre : Tout · Photos · Documents · Audio · Vidéos · Code. Ils s'activent aussi en tapant « photos », « vidéos »…, comme dans Quiera.
- **Deux groupes de résultats** :
  - **Par le nom** en premier, instantané ;
  - **Par le contenu** ensuite, qui arrive quelques millisecondes après.
- **L'indexation visible mais discrète** : un petit anneau de progression dans la barre (« 12 400 fichiers sur 58 000 »), en pause sur batterie.
- **Glisser un résultat** vers une autre appli (Explorateur, Premiere, Discord…) ; Ctrl+C copie le fichier.
- **Une fenêtre de réglages** au même style que celle de Quiera : barre personnalisée avec seulement Réduire et Fermer, choix des dossiers, du raccourci et du modèle, et l'espace pris par l'index.

### Ce qu'on laisse de côté

Les onglets de l'île, la musique, les notifications et les aperçus système de Quiera : l'appli fait **une seule chose**, chercher.

---

## 8. Ce qu'on reprend du code de Quiera

Le dépôt de Quiera est dans `D:\nom a trouver` (GPL-3.0 ; si la nouvelle appli prend une autre licence, il faudra réécrire ce qu'on reprend ou garder la GPL-3.0).

| Besoin | Dans Quiera |
|---|---|
| Barre de recherche, résultats, clavier | `src/modules/launcher/Launcher.tsx`, `results.ts`, `search.ts`, `launcher.css` |
| Forme et animation de l'île | `src/island/geometry.ts`, `springs.ts`, `IslandShell.tsx`, `island.css` |
| Couleurs et polices | `src/styles/tokens.css` |
| Index des noms de fichiers, types de fichiers (« images », « vidéos »…) | `src-tauri/src/files.rs` (`kind_of`, `kind_word`, `parse_query`) |
| Raccourci clavier global, fenêtre du lanceur | `src-tauri/src/platform/windows/launcher.rs`, `key_tap.rs` |
| Miniatures Windows | `src-tauri/src/platform/windows/thumbnail.rs` |
| Ouvrir un fichier ou son dossier | `src-tauri/src/platform/windows/process.rs` |
| Fenêtre de réglages avec barre personnalisée | `src/settings/SettingsApp.tsx`, `settings.css` |
| Français et anglais | `src/i18n/` |
| Publication et mises à jour signées | `.github/workflows/release.yml`, `docs/releases/` |
| Site (Vite + React, pages pré-rendues, guides, pages légales) | `site/` |

---

## 9. Première étape proposée : un prototype de mesure (pas d'interface)

Un petit programme Rust en ligne de commande qui :

1. charge EmbeddingGemma 2 (GGUF) avec llama.cpp, sur la carte graphique puis sur le processeur ;
2. indexe 1 000 photos, 100 PDF et 10 minutes d'audio d'un dossier de test ;
3. répond à des recherches comme « chien sur la plage » ou « facture d'électricité » ;
4. affiche le temps par fichier et la place prise.

But : savoir si la qualité et la vitesse sont bonnes **sur ton PC** avant de construire l'appli.

Ensuite seulement : l'appli Tauri avec le design ci-dessus, puis l'indexation en arrière-plan, puis l'installateur.

---

## 10. Résultats du prototype (8 novembre 2026)

Prototype Rust en ligne de commande : `proto/` (`furet-proto index | cherche | stats`). Mesures sur le PC de développement : RTX 4070 Laptop (8 Go), i7-12700H, 16 Go.

### Ce qui est installé (tout dans `D:\furet`, rien dans Windows)

- `outils/llama-vulkan` et `outils/llama-cpu` : llama.cpp b12202.
- `modeles/` : EmbeddingGemma 2 Q8_0 (296 Mo) + projecteur image et son (529 Mo) ; Parakeet TDT v3 int8 (≈ 640 Mo décompressé).
- `outils/pdfium` (pdfium.dll), `outils/sherpa-onnx` (serveur de transcription).

### Ce qui marche

- **Tous les types** : photos (WIC, orientation EXIF), PDF (pdfium, OCR des pages scannées), Word, PowerPoint, OpenDocument, Excel, CSV, texte, HTML, code, audio (tranches de 30 s), vidéo (plans + son).
- **OCR de Windows** (français) sur les photos, les captures et les scans.
- **Transcription** Parakeet v3 : très bonne en français (ponctuation, nombres). On retrouve un audio par ce qui y est dit (« dit 00:00–00:30 »).
- **Recherche entre types** : « facture d'électricité » ramène à quasi-égalité la photo, le PDF scanné et l'audio de la facture. Sur nos fichiers de test, le bon fichier arrive premier presque à chaque fois.
- Préfixes de Google : `task: search result | query: …` pour la recherche, `title: … | text: …` pour les documents ; rien pour image, son et vidéo.

### Vitesse (carte graphique, PC au repos)

- 200 fichiers (150 photos 12 Mpx, 30 PDF de 6 pages, 20 textes) : **21 s**, OCR compris, soit environ 550 fichiers par minute. C'était 70 s avant optimisation.
- Une recherche : 10 à 40 ms (dont 2 ms dans l'index de test, en 256 dimensions).
- **Index plus gros (tout un PC)**, sqlite-vec exhaustif en 256 dimensions : 66 ms à 50 000 morceaux, 273 ms à 200 000, **666 ms à 500 000** (500 Mo). En int8 : 440 ms ; en binaire : 140 ms mais moins précis. **L'appli doit donc passer à un index HNSW (usearch)** au-delà de ~100 000 morceaux.
- Processeur seul : environ 15 fois plus lent que la carte graphique (photo ≈ 4,5 s).
- Transcription : environ 9 fois le temps réel sur le processeur, une fois réglée (2 × 4 cœurs). Seule la parole est transcrite. C'est le plus lent : 1 h de parole ≈ 7 min.
- Dossier réel de l'utilisateur (328 fichiers : photos, audio, vidéo, texte, code), avant les deux passes : 2 min 17, dont les deux tiers en transcription.

### Réglages retenus

- **140 jetons par image** (`--image-max-tokens 140`) : 1,5 fois plus rapide que 280, sans perte visible sur nos tests. À vérifier sur de vraies photos.
- **Préparation en parallèle** (5 fils : décodage, OCR, ffmpeg) pendant que la carte graphique calcule ; lots de 8 morceaux mélangeant plusieurs fichiers ; 2 lots en vol.
- **Vidéo** : on ne décode que les images clés quand elles sont espacées de 2 s au plus (6,5 fois plus rapide), sinon tout. Plans repérés par comparaison d'images 32×18 ; au plus 8 images par plan.
- **Vulkan plutôt que CUDA** : CUDA n'est pas plus rapide ici (essayé puis supprimé) et ajoute 570 Mo.
- **Recherche par nom** : tous les fichiers du dossier (tous types, même `.zip` ou `.exe`), sans accents ni majuscules, mots dans le désordre, une faute de frappe tolérée (deux pour les mots longs). Affichée au-dessus des résultats par le contenu. Moins de 1 ms pour quelques milliers de noms.
- **Transcription en deuxième passe** : tout est cherchable d'abord, la parole est transcrite ensuite.
- **Détecteur de parole sans modèle en plus** : le vecteur son d'EmbeddingGemma est comparé à « une personne qui parle » et à « de la musique sans paroles » (et autres). On transcrit si l'écart dépasse -0,035. Mesuré : parole de -0,011 à +0,076 (même avec de la musique derrière), musique, bruit et bip de -0,045 à -0,129. Sur nos tests, 9 tranches sur 16 transcrites, sans aucune erreur.
- **256 dimensions** (Matryoshka) au lieu de 768 : index 3 fois plus petit, recherche 3 fois plus rapide, et classement même un peu meilleur une fois les « aimants » corrigés.
- **Correction des « aimants » et seuil** : certains vecteurs (surtout les sons) ressemblent un peu à tout. Chaque morceau reçoit à l'indexation un biais (moyenne de ses 10 meilleures ressemblances à 40 phrases génériques), retranché du score à la recherche. Seuil affiché **0,715**. Mesuré sur 31 recherches dont 8 sans réponse : précision 0,84 → 0,97 ; les recherches sans réponse (« match de football », « une voiture rouge »…) n'affichent plus rien.
- **Sur processeur : 70 jetons par image** (2,7 fois plus rapide qu'à 280).
- **Tout un PC** : contenu ignoré pour les dossiers système (à la racine d'un disque), AppData, bibliothèques de jeux (steamapps, Epic, Riot…), dossiers cachés ou système, node_modules, dossiers de compilation ; images de moins de 200 pixels (icônes) ; sons de moins de 16 Ko. Leurs noms restent cherchables.
- **Pensé pour tous les PC** : priorité basse pour tout (le PC reste fluide). Fils de préparation selon le nombre de cœurs (2 à 6). Transcription adaptée : 1 × 2 cœurs sur un petit PC, jusqu'à 3 × 4 cœurs. Parakeet ne va pas plus vite au-delà de 4 cœurs par transcription.

### Mémoire et place

- llama-server : environ 1,5 Go de mémoire vive et 2,2 Go sur la carte graphique. Le programme lui-même : 30 à 370 Mo, selon l'OCR et les vidéos.
- Index : environ 3 Ko par morceau en 768 dimensions (la réduction à 256 dimensions diviserait par 3, pas encore essayée).

### Limites connues

- Les sons sans parole (bip, bruit) sont mal retrouvés.
- Les scores des résultats sans rapport tournent autour de 0,60 : il faudra un seuil ou un tri par type.
- Le premier lancement sur une carte graphique coûte jusqu'à une minute de préparation de Vulkan (une seule fois).
- Le seuil et le détecteur de parole sont réglés sur des fichiers de test (en partie synthétiques) : à surveiller sur de vrais dossiers.

### Dans l'appli (étape 4, sur le PC de l'utilisateur, 125 888 morceaux)

- **Index rapide HNSW (usearch, vecteurs en 16 bits)** : **0,38 ms** par recherche contre 229 ms en exhaustif ; rappel à 10 de 96 % (plafond dû aux ex æquo : fichiers et images de vidéo identiques) ; 145 Mo de mémoire ; construit en 6,4 s sur tous les cœurs (46 s sur un seul), puis sauvegardé et rechargé au lancement.
- **Mots exacts (FTS5)** : un numéro de facture retrouve le PDF, son scan et ses photos (OCR) ; un mot dit dans un audio retrouve l'audio (transcription). Les deux classements sont fusionnés (« reciprocal rank fusion ») ; l'extrait s'affiche avec les mots en gras.
- **Recherche avec le serveur texte seul sur le processeur** : 38 ms, 270 Mo, la carte graphique reste au repos.

### Noms de tous les disques (étape 5, C: 917 Go + D: 2 To)

- Sans exclusions : 8 millions de noms, 164 s, 535 Mo, et jusqu'à 31 s pour une recherche tolérant une faute. En excluant ce que personne ne cherche (Windows, ProgramData, AppData, node_modules, dossiers cachés, steamapps…) : **1,84 million de noms, 12 à 15 s de parcours, 116 Mo**, rechargés en **0,14 s** au lancement.
- Recherche : **3 à 5 ms** (le mot cherché dans un seul grand texte des clés, sur 8 cœurs) ; la tolérance aux fautes ne porte que sur les fichiers lus par leur contenu (~30 000).
- Suivi en direct sans administrateur (`ReadDirectoryChangesW` sur chaque disque) : un fichier créé est trouvable dans la seconde ; un fichier supprimé disparaît.
- La MFT et le journal USN (comme Everything) demandent les droits administrateur : un service, installé avec l'appli (étape 8).
- À affiner : pour un mot inventé, quelques fichiers de code passent de justesse le seuil de la recherche par le contenu.
- Pas encore faits : tree-sitter pour le code, vieux `.doc` et `.ppt`, index des mots exacts (FTS5), réduction des vecteurs, pause sur batterie et pendant les jeux.

### Visages et dossiers (après l'étape 5)

- Visages (`moteur/src/visages.rs`) : détection **YuNet** (MIT) puis empreinte **SFace** (Apache 2.0, 128 nombres), modèles ONNX dans `modeles/visages/`, lus par **tract** (Rust pur : marchera aussi sous Linux). Une passe après la lecture, sur 2 à 6 fils en priorité basse : **~0,1 à 0,16 s par photo** sur le processeur.
- Une photo avec un seul visage dont le nom ressemble à un prénom (« elliot.png », « Elliot Martin.jpg ») donne un nom à ce visage. Chercher « elliot » trouve alors les autres photos où ce visage est reconnu (seuil 0,40) : test sur 5 photos, même personne +0,73 à +0,80, personnes différentes ≤ +0,29.
- Dossiers : par le contenu, les 40 premiers fichiers trouvés sont regroupés ; un dossier qui en contient au moins 3 est proposé (« 5 fichiers correspondants »), au plus 2. Par le nom, les dossiers étaient déjà trouvés (liste des disques).

### Aperçus, filtres et actions (étape 6)

- Aperçu dans l'île, à la place de la liste : photo réduite par WIC (orientation EXIF, HEIC, RAW), page de PDF rendue par pdfium, vidéo et son lus par la WebView à partir du moment trouvé (le fichier est servi par plages de 4 Mo ; un format qu'elle ne lit pas montre sa grande miniature), texte autour du passage avec les mots surlignés, liste d'un dossier ou d'une archive.
- Filtres : analysés en Rust (testés), retirés du texte cherché, appliqués aux deux recherches. Par le nom, 400 noms sont regardés pour en garder 4 (taille et date lues sur le disque seulement si un filtre les demande). Une recherche faite seulement de filtres liste les fichiers lus les plus récents.
- Glisser : `SHDoDragDrop` dessine l'image du fichier ; la bibliothèque `drag` (CrabNebula) aurait ajouté une deuxième version du paquet `windows`.

### Archives et lieux (étape 7)

- .zip : au plus 300 fichiers, 300 Mo en tout et 50 Mo par fichier ; pas de sons, de vidéos ni d'archives dans l'archive. Test : 4 fichiers lus en 0,27 s ; « ferry pour la Corse » trouve « dans notes/idees_vacances.txt ».
- Lieux : 31 815 villes de plus de 15 000 habitants (sans les quartiers PPLX), 2,7 Mo, avec leurs autres noms en écriture latine (« Londres », « Barcelone ») et les noms des pays en français (CLDR). Un arrondissement est rattaché à sa ville (une ville 5 fois plus peuplée à moins de 5 km) ; une grande ville (500 000 habitants) à moins de 30 km donne aussi son nom (une photo à Boulogne-Billancourt sort pour « Paris »). Les noms de régions sont ceux de GeoNames (souvent en anglais : « Catalonia »).
- Un nom de lieu ne compte que si des photos de l'index y ont été prises : « orange » n'est un lieu que pour qui a des photos d'Orange. Les photos du lieu passent en tête, sans filtrer le reste.

### Pertinence et fluidité (10 novembre)

- Un mot inventé (« zorglub », « xkqzv ») faisait sortir des fichiers de code juste au-dessus du seuil (0,75 à 0,78). La forme des scores (écart du premier à la moyenne) ne sépare pas ces recherches des vraies. Ce qui les sépare : la ressemblance de la recherche au vecteur moyen de 12 mots inventés — 0,83 à 0,87 pour les mots inventés, 0,72 à 0,81 pour 13 vraies recherches. Le seuil monte de 2,5 fois l'excès au-dessus de 0,80 : plus aucun résultat pour les mots inventés, « logo », « azerty », « un chat » gardent les leurs (exemple `moteur/examples/calibrer_requetes.rs`).
- Par le nom : 40 candidats reclassés (fichiers ouverts depuis Furet, demi-vie 30 jours ; dossiers indexés +0,04 ; dossier de l'utilisateur +0,02 ; nom exact +0,05), sans rien lire sur le disque (un disque dur en veille mettrait des secondes à répondre).
- Les commandes synchrones de Tauri tournent sur le fil de l'interface : la recherche par le nom passe sur un fil à part.

### Premier lancement et installateur (étapes 8 et 9)

- Téléchargements : 1,1 Go pour l'essentiel, + 508 Mo pour la transcription, + 39 Mo pour les visages. Essai sur un dossier vide : 11 paquets, 1,5 Go en 237 s, tous vérifiés ; indexation (photos, archive, son, vidéo, visages, lieux, transcription) et recherche OK depuis ce dossier. Premier chargement du modèle sur la carte graphique : 28 s (Vulkan prépare ses programmes une fois).
- Archives : on ne garde que llama-server et ses DLL (pas les 20 autres outils de llama.cpp), ffmpeg et ffprobe (pas ffplay), le serveur websocket de sherpa-onnx.
- Les villes sont mises en forme sur le PC (`lieux::preparer`, comme le script Python d'origine) à partir de GeoNames et du CLDR : ces fichiers changent chaque jour, ils ne sont donc pas vérifiés par empreinte.
- Installateur NSIS de 11 Mo, installation pour l'utilisateur seul (pas de droits administrateur). Mises à jour signées (minisign, tauri-plugin-updater).

---

## Sources

- DigUp : https://github.com/ARahim3/DigUp
- EmbeddingGemma 2 : https://huggingface.co/google/embeddinggemma-2 · https://ai.google.dev/gemma/docs/embeddinggemma/model_card_2
- llama.cpp et EmbeddingGemma 2 : https://huggingface.co/ggml-org/embeddinggemma-2-GGUF · https://github.com/ggml-org/llama.cpp/issues/30082
- Qwen3-VL-Embedding : https://arxiv.org/pdf/2601.04720 · https://huggingface.co/Qwen/Qwen3-VL-Embedding-2B
- Transcription (Whisper, Parakeet) : https://www.gladia.io/blog/best-open-source-speech-to-text-models · https://snailtext.app/blog/parakeet-vs-whisper-turbo-vs-qwen3-asr/
- ONNX Runtime, DirectML et Windows ML : https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- Bases de vecteurs : https://aloa.co/ai/comparisons/vector-database-comparison/best-embedded-vector-databases
- MFT et journal USN : https://docs.rs/crate/ntfs-reader/latest
- Documents en Rust : https://docs.rs/crate/mdkit/latest · https://docs.rs/crate/kreuzberg/latest/source/README.md
- OCR : https://hackernoon.com/c-ocr-libraries-the-definitive-net-comparison-for-2026
- Recherche Windows sur Copilot+ : https://www.windowscentral.com/hardware/laptops/qualcomm-snapdragon-pcs-first-to-get-long-awaited-semantic-search-in-latest-windows-11-insider-update
- Villes du monde : GeoNames (https://www.geonames.org, CC BY 4.0) · noms des pays : Unicode CLDR (https://cldr.unicode.org)
- Concurrents payants : https://www.filescope.dev/ · https://beaconfind.app/ · https://localspider.com/ · https://github.com/Ishannaik/pixafind-releases · https://filect.io/blog/best-ai-file-management-tools/
