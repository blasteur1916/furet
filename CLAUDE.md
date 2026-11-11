# Furet (nom provisoire)

Appli Windows de recherche de fichiers **par leur contenu** (photos, PDF, Office, audio, vidéo, code) et **par leur nom**, dans une seule barre, entièrement en local. Elle remplace Quiera (une Dynamic Island pour Windows, arrêtée le 8 novembre 2026), dont le dépôt est dans `D:\nom a trouver`.

Tout le contexte est dans [RECHERCHE.md](RECHERCHE.md) :
- le modèle (EmbeddingGemma 2 via llama.cpp) ;
- la techno pour chaque type de fichier ;
- la concurrence (surtout DigUp) ;
- le design, qui reprend la barre de recherche de Quiera ;
- le code de Quiera à réutiliser ;
- la première étape.

## Façon de travailler

- Répondre **en français**, simplement.
- Avancer **étape par étape** et faire valider chaque étape avant la suivante.
- Commit et push seulement quand c'est demandé.
- Ne jamais publier d'infos personnelles ni de secrets ; ne jamais écrire le contenu des fichiers de l'utilisateur dans les journaux.

## Où on en est

Le **prototype Rust en ligne de commande** est fait : `proto/` (`furet-proto index <dossier>`, `cherche "…"`, `stats`). Il indexe tous les types de fichiers (photos, PDF, audio, vidéo, Office, tableurs, texte, code) avec EmbeddingGemma 2 (llama-server, Vulkan), l'OCR de Windows et la transcription Parakeet v3 (sherpa-onnx), en deux passes : d'abord tout est cherchable, puis la parole est transcrite. Il cherche aussi **par le nom** dans tous les fichiers (tous types). Résultats et réglages : partie 10 de [RECHERCHE.md](RECHERCHE.md).

L'utilisateur l'a testé sur un vrai dossier (328 fichiers) : ça marche. Seuil réglé (correction des « aimants », 256 dimensions). Le prototype est terminé.

**Le vrai logiciel** est commencé (licence **GPL-3.0**, comme Quiera, dont l'utilisateur est l'auteur). Plan en 9 étapes validé, chacune à faire valider :
1. squelette : l'île et la barre de Quiera, Alt+Espace, recherche dans l'index existant — fait ;
2. résultats riches (miniatures Windows, ligne « pourquoi ») — fait ;
3. indexation en arrière-plan dans l'appli (dossiers et options dans les réglages, roue à droite de la barre ; pause sur batterie, jeu, plein écran ou à la demande, qui arrête aussi le llama-server d'indexation) — fait ;
4. index rapide HNSW (`moteur/src/vecteurs.rs`, usearch, tenu à jour par un journal que remplissent des déclencheurs SQLite) + mots exacts (FTS5, table `textes`) + recherche mixte avec extraits — fait ;
5. noms de tous les disques (`moteur/src/disque.rs`) : parcours rapide sans droits administrateur, liste compacte sauvegardée (`donnees/noms-disques.bin`), suivi en direct par `ReadDirectoryChangesW` — fait, **à valider**. La MFT (plus rapide, mais administrateur) viendra avec un service installé à l'étape 9 ;
   En plus : **visages** (`moteur/src/visages.rs`, YuNet + SFace avec tract ; « elliot.png » nomme un visage, chercher « elliot » trouve ses autres photos ; réglage « Reconnaître les visages ») et **dossiers** dans les résultats par le contenu — fait, **à valider** ;
6. aperçus, filtres et actions — fait, **à valider** :
   - aperçu (`app/src-tauri/src/apercus.rs`, `app/src/barre/Apercu.tsx`) avec Espace après les flèches, Maj+Espace ou l'œil : photo, page du PDF (← →), vidéo ou son lu au moment trouvé, texte autour du passage, contenu d'un dossier ou d'une archive ; fichiers servis par `http://furet-fichier.localhost/<clé>` (plages pour les vidéos) ;
   - filtres tapés dans la barre (`moteur/src/filtres.rs`, testé) : type, date (« 2024 », « mars 2023 », « hier », « la semaine dernière », « avant 2020 »…), taille (« > 1 Go ») ; montrés en pastilles ; un type seul (« photos ») reste une recherche de nom ;
   - actions (`app/src-tauri/src/actions.rs`) : copier le chemin (Ctrl+C), montrer dans le dossier (Ctrl+Entrée), ouvrir avec (Ctrl+O, `SHOpenWithDialog`), glisser le fichier vers une autre appli (`SHDoDragDrop`, copie jamais déplacement) ;
7. nouvelles sources — fait, **à valider** :
   - archives .zip (`Genre::Archive`) : les photos, PDF, documents, textes et code qu'elles contiennent sont lus (copiés un instant dans un dossier temporaire) ; le résultat dit « dans notes/idees.txt » (colonne `morceaux.partie`) ;
   - lieux et dates des photos (`moteur/src/lieux.rs`) : GPS et date de prise de vue lus dans l'EXIF (kamadak-exif), ville la plus proche d'après `modeles/lieux/` (GeoNames, CC BY 4.0 : à citer dans « À propos »), table `lieux` ; « vacances Espagne », « photos Paris » mettent les photos prises là en tête ; « photos 2019 » utilise la date de prise de vue ;
   Optimisation (10 novembre, **à valider**) : seuil relevé pour les recherches qui ressemblent à des mots inventés (`pertinence::Charabia`), noms reclassés (fichiers souvent ouverts — `app/src-tauri/src/historique.rs`, `historique.json` —, dossiers de l'utilisateur, nom exact), « Récents » dans la barre vide, copies d'un même fichier fusionnées, recherche par le nom hors du fil de l'interface, recherches dépassées abandonnées, vecteurs des dernières recherches gardés, aperçu préparé à l'avance, surbrillance qui glisse ;
8. premier lancement — fait, **à valider** :
   - `moteur/src/installation.rs` : 11 paquets (llama.cpp b12202 processeur et Vulkan, EmbeddingGemma 2 et son mmproj, PDFium chromium/8138, FFmpeg 9.0.2, villes GeoNames ; Parakeet + sherpa-onnx si la transcription est voulue ; YuNet + SFace pour les visages), vérifiés par SHA-256, reprise des téléchargements coupés, seuls les fichiers utiles gardés ; essai complet : 1,5 Go en 4 min, puis index et recherche OK ;
   - dossier de Furet (`app/src-tauri/src/installation.rs`) : `FURET_RACINE`, sinon `racine` dans `reglages.json` (sur le PC de l'utilisateur : `D:\furet`), sinon `%LOCALAPPDATA%\app.furet` ; ffmpeg pris dans `outils/ffmpeg` (`processus::ffmpeg`) ;
   - accueil (`app/src/accueil/Accueil.tsx`) ouvert tout seul tant que l'essentiel manque : dossiers, options (avec leur poids), démarrage avec Windows, bouton « Télécharger et commencer », avancement ; la recherche et l'indexation attendent ; une option activée plus tard fait télécharger ses modèles en arrière-plan ;
   - réglages : démarrage avec Windows, raccourci actuel, « À propos » (licences, GeoNames, conditions de Gemma), défilement ;
9. installateur et mises à jour — fait, **à valider** :
   - `pnpm tauri build` dans `app/` → `target\release\bundle\nsis\Furet_<version>_x64-setup.exe` (11 Mo, installation sans droits administrateur, sans les modèles) ;
   - mises à jour (`app/src-tauri/src/mises_a_jour.rs`, tauri-plugin-updater) : vérifiées 1 min après le lancement puis toutes les 6 h, téléchargées et vérifiées en arrière-plan, installées par « Redémarrer pour mettre à jour » dans les réglages. Clé privée de signature : `%USERPROFILE%\.tauri\furet-mises-a-jour.key` (hors du projet, sans mot de passe ; ne jamais l'afficher ni la publier ; la perdre empêche toute mise à jour). Construire avec `TAURI_SIGNING_PRIVATE_KEY` = ce chemin et `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` vide, puis `node scripts/latest.mjs "notes"` prépare `latest.json`. Page des versions : `github.com/blasteur1916/furet` (dépôt privé pour l'instant : les mises à jour demandent qu'il soit public) ;
   - démarrage avec Windows (tauri-plugin-autostart, argument `--demarrage`), réglage activé par défaut ;
   - pas encore : signature de l'installateur (certificat payant, décision de l'utilisateur ; sans lui, SmartScreen avertit) et service MFT (reporté : un service système qui lit tous les noms des disques demande des droits administrateur et une protection soignée, pour un gain faible).

**Linux** : l'utilisateur veut que Furet marche aussi sous Linux. Aujourd'hui le moteur et l'appli sont Windows seulement (WIC, OCR Windows, miniatures de l'Explorateur, Job Objects, RegisterHotKey, SetWindowRgn). Le nouveau code doit rester portable (`#[cfg]` par plateforme, comme `app/src-tauri/src/plateforme.rs`) ; le portage complet sera une étape à part.

### Organisation

Un espace de travail Cargo à la racine (`Cargo.toml`), compilé dans `target/` :
- `moteur/` : la librairie (indexation, recherche, OCR, transcription…) ;
- `proto/` : la ligne de commande (`target\release\furet-proto.exe index|cherche|stats`) ;
- `app/` : l'appli Tauri 2 + React (interface dans `app/src`, Rust dans `app/src-tauri`). Construire avec `pnpm tauri build --no-bundle` dans `app/` → `target\release\furet.exe` (sans `--no-bundle` : l'installateur aussi).

Les outils et modèles sont dans `outils/` et `modeles/`, l'index dans `donnees/index.sqlite` (partagé par le prototype et l'appli). Sur un autre PC, tout est dans `%LOCALAPPDATA%\app.furet`, téléchargé au premier lancement.

**Vérifier l'appli sans taper au clavier de l'utilisateur** (des touches envoyées pourraient partir dans sa fenêtre) : lancer `furet.exe` avec `FURET_TEST=1`, puis `furet.exe --ouvrir "texte"` (ou `--fermer`) ouvre la barre avec ce texte sans qu'elle se referme, et une capture du haut de l'écran montre le résultat. Ajouter `--apercu 2` ouvre l'aperçu du 3e résultat. En mode test, l'indexation reste en pause ; on peut pointer l'appli sur un index d'essai avec `FURET_RACINE` (un dossier avec `donnees\index.sqlite` et des jonctions vers `outils` et `modeles`). Relancer ensuite l'appli sans `FURET_TEST`.
