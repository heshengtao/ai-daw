<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW — une station de travail audio numérique locale, native pour l'IA">
</p>

<p align="center">
  <a href="https://github.com/heshengtao/ai-daw/releases"><img src="https://img.shields.io/github/v/release/heshengtao/ai-daw?style=flat-square&color=1A6CFF" alt="Version GitHub"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="Licence : Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux%20/%20Web-1A6CFF?style=flat-square" alt="Plateformes : Windows, macOS, Linux, Web">
  <a href="https://hub.docker.com/r/ailm32442/ai-daw"><img src="https://img.shields.io/docker/pulls/ailm32442/ai-daw?style=flat-square&color=1A6CFF" alt="Téléchargements Docker"></a>
</p>

<p align="center">
  <a href="README_zh.md">简体中文</a> · <a href="README_zh-TW.md">繁體中文</a> · <a href="README.md">English</a> · <a href="README_ja.md">日本語</a> ·
  <a href="README_ko.md">한국어</a> · <a href="README_es.md">Español</a> · <a href="README_fr.md"><b>Français</b></a> · <a href="README_de.md">Deutsch</a> ·
  <a href="README_ru.md">Русский</a> · <a href="README_ar.md">العربية</a>
</p>

<p align="center">
  <a href="https://hub.docker.com/r/ailm32442/ai-daw"><b>Docker Hub</b></a> ·
  <a href="https://github.com/heshengtao/ai-daw/releases">Télécharger</a>
</p>

## Qu'est-ce qu'AI DAW ?

**AI DAW est un petit studio de musique natif pour l'IA, installé sur votre bureau** — composez, arrangez, mixez et masterisez dans une seule application légère, et laissez un agent IA travailler dans la session à vos côtés.

C'est un véritable DAW : une timeline multipiste, un piano roll, une table de mixage avec départs et bus, des pistes d'automatisation, un synthétiseur intégré et l'hébergement de plugins VST3 / CLAP / AU. Il est également conçu pour être **piloté par un modèle** — un Skill fourni apprend à n'importe quel environnement d'IA à piloter l'application en cours d'exécution via MCP, si bien qu'un agent peut ajouter des pistes, écrire des notes, équilibrer un mixage et analyser le résultat pendant que vous le regardez se faire dans l'interface.

Tout s'exécute sur votre propre machine. Aucun compte, aucune clé API, aucun fichier audio ne quitte votre ordinateur.

<img src="./assets/screenshots/01-arrange.png" width="100%" alt="AI DAW — la vue Arrangement : six pistes de clips sur la timeline, l'inspecteur à droite affichant une chaîne EQ et réverbération, et une piste d'automatisation en bas">

## Pourquoi c'est important

Les modèles sont devenus bons en écriture de code parce qu'ils pouvaient **exécuter** ce code via un framework d'agent de codage ; ils sont devenus bons en modélisation 3D parce qu'ils pouvaient piloter un vrai logiciel de modélisation via Blender MCP. Dans les deux cas, le modèle dispose d'un vrai outil, d'un vrai résultat et d'une vraie boucle de rétroaction — et c'est ce qui rend l'itération rapide possible.

La musique a longtemps été privée de cette boucle. Un modèle peut générer de l'audio, mais il ne peut pas *travailler sur une session* comme le fait une personne : pas d'arrangement à éditer, pas de table de mixage à piloter, pas d'historique d'annulation, aucun moyen de remarquer que sa dernière modification a empiré les choses.

**AI DAW est une tentative modeste et délibérée de combler cet écart.** C'est un vrai DAW qu'un agent IA peut piloter grâce à un Skill et à MCP, ce qui le rend utile pour :

- **Tester ce qu'un modèle sait réellement de la composition de chansons** — non pas si son audio semble plausible, mais s'il peut construire un arrangement, choisir des parties et corriger un mixage.
- **Collecter de vraies traces de composition** — chaque modification de l'agent atterrit dans un projet normal avec un historique d'annulation normal, si bien qu'une session devient une donnée sur la façon dont un modèle raisonne à propos de la musique.
- **Un pas vers des modèles capables d'écrire de la musique comme ils écrivent du code** — avec un outil qui leur répond.

## Léger par conception

La légèreté est une fonctionnalité, pas un compromis.

- **Une empreinte légère** — pas d'Electron, pas de cloud, pas de zoo de modèles embarqué. L'application macOS fait environ 38 Mo — à titre de comparaison, les DAW grand public comme Ableton Live, Logic Pro et FL Studio s'installent généralement entre 1 et 3 Go, ce qui rend AI DAW **plus de 95 % plus petit**.
- **Démarrage rapide** — rien de lourd ne se charge au lancement ; les modèles IA ne sont chargés que lorsque vous les utilisez.
- **De gros modèles à la demande, jamais dans l'installateur** — les tâches lourdes comme la génération musicale ne sont pas intégrées par défaut. La **voix chantée** (SoulX-Singer, 1,4 Go) et l'**échange de voix** (SoulX-Singer-SVC, 2,4 Go) sont des modèles optionnels dans Gestionnaire → Modèles : téléchargez-les quand vous le souhaitez, et ils n'occupent aucun espace disque avant cela.
- **Conçu pour être étendu** — les **extensions d'application** (paquets npm) ajoutent déjà des outils d'agent et des panneaux latéraux ; les interfaces permettant de brancher des modèles IA externes dans le flux de travail sont sur la feuille de route.

## Pilotez-le depuis n'importe quel environnement d'IA

AI DAW fournit un **Skill** (`skills/aidaw`) qui apprend à un agent IA à piloter l'application en cours d'exécution — lire et écrire le projet, les pistes, les clips et les notes ; analyser un style, consulter la théorie musicale et les motifs de batterie, et transformer une recette de genre en MIDI ; mixer (gain, panoramique, départs, automatisation) ; gérer la lecture et l'enregistrement ; analyser la sonie et le spectre ; exporter en WAV ; scanner les plugins. Il fonctionne via le serveur **MCP** intégré, si bien que tout environnement prenant en charge les Agent Skills peut l'adopter.

L'application embarque aussi 18 instruments échantillonnés — **piano acoustique, guitares nylon et électrique clean, basse finger, batterie acoustique, ensemble à cordes, cordes lentes, violon/alto/violoncelle/contrebasse solo, cordes pizzicato, trompette, cors d'harmonie et cordes/cuivres de synthé 1 et 2** (SF3/SF2, environ 13,5 Mo au total) — prêts à jouer dès l'installation.

**Option 1 · Installation depuis la ligne de commande**

```bash
npx skills add heshengtao/ai-daw
```

**Option 2 · Demandez à votre agent de l'installer**

```
Install this Skill: https://github.com/heshengtao/ai-daw
```

Chaque modification effectuée par un agent passe par le même pipeline de commandes que l'interface : elle entre dans l'historique d'annulation, apparaît dans l'interface et est attribuée à l'agent — vous pouvez donc toujours voir ce qu'il a fait et l'annuler. Vous préférez rester dans l'application ? **Écrire avec l'IA** ouvre la même boucle sous forme de panneau de discussion (Paramètres → Génération de texte).

## Fonctionnalités

### Inspire — Une ligne en entrée, une prise aboutie en sortie

La vue la plus à gauche de la barre supérieure est **Inspire** : un bureau pleine page « écrire → générer → écouter ». Vous écrivez à gauche, vous écoutez à droite. Pas de likes, pas de partages, pas d'accès payant — seulement votre description, vos paroles et la barre de lecture.

- **Un seul menu déroulant, deux moteurs** — les poids locaux YuE2 / ACE-Step et tout service cloud configuré (Suno / Mureka / Lyria …) partagent une même liste ; ajouter un fournisseur ne demande qu'une modification à un seul endroit
- **Simple / Avancé** — le mode Simple se contente d'une ligne de description ; le mode Avancé vous laisse écrire des paroles avec les balises `[Verse]` / `[Chorus]` / `[Bridge]` / `[Outro]`, basculer l'interrupteur **Instrumental** et régler la durée, les étapes et la graine
- **Prises par lot** — demandez jusqu'à 8 prises en une seule pression ; elles sont rendues l'une après l'autre, chacune avec une nouvelle graine, et apparaissent dans la liste au fur et à mesure
- **Prolonger une prise** — chaque rendu local conserve un code sémantique, si bien que **Prolonger** écrit le segment suivant à partir du point où la prise s'est arrêtée — le modèle reçoit la structure, pas seulement l'audio
- **Il récupère ce dont il a besoin** — des poids manquants, un moteur absent ou un moteur arrêté sont traités dans l'ordre lorsque vous appuyez sur Générer
- **Les prises en colonne** — regroupées par jour ; chaque prise reçoit une **pochette générée** aux couleurs des pistes du projet, un bouton de lecture toujours visible et sa propre barre de lecture : cliquez pour écouter, faites glisser pour parcourir. La carte sélectionnée est aussi celle que pilote la touche de transport de la barre inférieure
- **À vous de garder** — l'historique vit sur votre machine ; le nettoyer ne touche jamais au projet, et une prise n'entre dans la session (＋) ou n'atteint le disque (enregistrer) que lorsque vous le demandez

<p align="center">
  <img src="./assets/screenshots/02-inspire.png" width="100%" alt="La page Inspire : le composeur à gauche en mode Avancé avec une description de style, des paroles et des prises, et l'historique de génération à droite avec les pochettes générées et les barres de lecture">
</p>

### Arrangement et édition — Composez sur une vraie timeline

Disposez un morceau tel que vous l'entendez. Faites glisser des clips d'une piste à l'autre, découpez-les là où la musique change, et laissez la règle et la grille suivre un tempo et une signature rythmique que vous pouvez modifier n'importe où dans le morceau. Les clips MIDI dessinent leurs propres notes et les clips audio leur propre forme d'onde : vous lisez l'arrangement d'un coup d'œil.

- **Arrangement multipiste** — clips MIDI et audio côte à côte, glisser-déposer entre pistes, sélection au lasso
- **Un piano roll lisible** — les aperçus de clips affichent les notes ; l'éditeur vous donne la vélocité, le zoom et un clavier virtuel
- **Partition et tablature** — la piste de partition dessine une portée ou une tablature de guitare à partir des mêmes notes, et le MusicXML est produit à l'export
- **Tempo et mesure dynamiques** — changez le tempo ou la signature rythmique à tout moment ; la grille suit
- **Transport et boucle** — lecture, arrêt, navigation, bouclage d'une région, métronome
- **Pistes d'automatisation** — dessinez des courbes pour tout paramètre exposé par un plugin ou par le synthétiseur intégré
- **Des outils au bout du curseur** — un dock flottant bascule entre dessin / gomme / sélection / découpe / paroles, et un clic fait passer la piste inférieure de l'automatisation à la partition ou au clavier
- **Une vraie boîte à outils d'édition** — découpe, déplacement, copier/coller, annuler/rétablir, plus une aide-mémoire des raccourcis clavier

<p align="center">
  <img src="./assets/screenshots/03-edit.png" width="100%" alt="L'éditeur piano roll : les notes d'un clip de mélodie avec la piste de vélocité en dessous, et la chaîne EQ et réverbération de la piste dans l'inspecteur">
</p>

### Mixage et mastering — Finalisez le morceau

Équilibrez le morceau, puis faites-le sonner fort. Les tranches de console offrent le gain, le panoramique, les départs, les bus, le muet/solo et le vumètre ; une chaîne de mastering sur le bus master gère la sonie, les crêtes et la largeur stéréo, pour produire un rendu prêt à sortir sans quitter l'application.

- **Table de mixage** — gain, panoramique, départs, bus, muet/solo et vumétrie
- **Effets intégrés** — dynamique, EQ et effets temporels, prêts à être déposés sur n'importe quelle piste
- **Chaîne de mastering** — normalisation de la sonie, limitation true peak et largeur stéréo
- **Synthétiseur intégré** — oscillateurs, enveloppes et un filtre avec oscilloscope en direct
- **Export** — WAV sans perte ou MP3 encodé par LAME (fréquence d'échantillonnage et débit de votre choix), un fichier MIDI standard ou une partition MusicXML

<p align="center">
  <img src="./assets/screenshots/04-mix.png" width="100%" alt="La vue Mixage : les tranches de console du bus master, de cinq pistes d'instruments et d'un bus de réverbération, avec faders, panoramiques, départs, inserts et vumètres en direct">
</p>

### Plugins — Amenez vos propres instruments et effets

AI DAW héberge des plugins **VST3** et **CLAP** partout, ainsi que des **Audio Units (AU / AUv3)** sur macOS : les outils que vous possédez déjà vous accompagnent. Un gestionnaire de plugins parcourt le registre ouvert, filtre par type et par compatibilité avec le système, et installe, met à jour ou supprime des plugins en un clic — ou pointez-le vers un plugin déjà présent sur votre disque.

- **Hébergement VST3 / CLAP / AU** — chargez des instruments et effets tiers sur n'importe quelle piste (AU et AUv3 sur macOS)
- **Gestionnaire de plugins** — recherchez dans le registre, filtrez par instrument/effet et par plateforme, installez / mettez à jour / désinstallez
- **Import local** — importez un plugin que vous avez installé ailleurs
- **Éditeurs et paramètres** — ouvrez l'interface propre d'un plugin, ou pilotez ses paramètres depuis l'inspecteur et les pistes d'automatisation ; les plugins AU sans vue propre reçoivent un panneau de paramètres générique
- **Bibliothèques SoundFont** — importez vos propres `.sf2` / `.sf3` (une banque se déploie en dizaines de préréglages), ou installez une banque ouverte comme MuseScore General, FluidR3 ou FreePats depuis l'index SoundFont intégré
- **Un seul gestionnaire de téléchargements** — plugins, SoundFonts, modèles IA et runtimes partagent un même gestionnaire avec progression en direct, annulation et état d'installation par élément

<p align="center">
  <img src="./assets/screenshots/05-plugins.png" width="100%" alt="Le panneau Plugins : la bibliothèque SoundFont et les instruments intégrés regroupés par famille, avec des filtres intégré / externe">
</p>

### Extensions d'application — Étendez l'application elle-même

Les plugins audio étendent le son ; les **extensions d'application** étendent l'application. Elles sont distribuées sous forme de paquets npm et peuvent ajouter des **outils MCP** qu'un agent peut appeler, ainsi que des **panneaux** affichés dans la barre latérale droite — un flux de travail qui vous manque peut donc être ajouté sans forker l'application.

- **Distribution npm** — installez depuis un index (`index.json`) ou directement depuis un registre ; `engines.aidaw` indique les versions de l'application prises en charge par une extension, et des miroirs couvrent les réseaux hors ligne ou protégés par un pare-feu
- **Découvrir et Installé** — le panneau liste tout ce que vos index connaissent (version, auteur, installer / mettre à jour) à côté de tout ce qui s'exécute (état, outils, panneaux, activer / recharger / désinstaller)
- **Deux faces d'une extension** — les outils côté serveur s'exécutent dans l'hôte QuickJS du démon et atteignent la session via les mêmes commandes que l'interface ; les panneaux côté client sont des ESM chargés dans la webview avec rechargement à chaud
- **Dossiers locaux aussi** — Importer un dossier installe une extension que vous développez vous-même ; `examples/extensions/hello` est la référence minimale (un outil, un panneau)
- **Pas la chaîne audio** — pour VST3 / CLAP / AU, voir **Plugins** ci-dessus ; il s'agit de l'application, pas de la table de mixage

<p align="center">
  <img src="./assets/screenshots/06-extensions.png" width="100%" alt="Le panneau Extensions d'application : la liste Découvrir avec versions, auteurs et boutons d'installation, la pastille d'état de l'index, et les onglets Installé / Découvrir">
</p>

### IA locale — Transformez l'audio en matière exploitable

Les fonctions d'IA sont conçues pour de vraies sessions, pas pour des démos. Pointez-les sur une piste et elles vous rendent quelque chose que vous pouvez continuer à éditer : des notes MIDI déplaçables, une prise vocale moins chargée en bruit de pièce, des stems que vous pouvez rééquilibrer. Tout s'exécute sur votre propre matériel — CPU par défaut, avec accélération optionnelle CoreML (macOS) et DirectML (Windows). Chaque tâche figure dans un seul **panneau IA**, filtré par **Tout / Générer / Convertir**, et chaque ligne indique si son modèle est déjà installé.

- **Détection de tempo** — Beat This! (ISMIR 2024) déduit un BPM global et une signature rythmique en une seule passe
- **Audio vers MIDI** — Basic Pitch transforme une interprétation enregistrée en notes éditables ; passez le modèle à **MuScriptor** (via le moteur audio.cpp) pour une transcription multi-instruments, qui produit une piste MIDI par instrument. 393 Mo–5,1 Go, téléchargé à la demande (CC BY-NC 4.0 — non commercial)
- **Nettoyage de voix** — LocalVQE réduit le bruit et la réverbération des prises vocales
- **Séparation de stems** — HTDemucs v4 découpe une piste en 4 ou 6 stems, prêts à être remixés
- **Voix chantée** — une mélodie MIDI, des paroles et une voix de référence deviennent une prise vocale éditable (clonage de timbre zero-shot de SoulX-Singer). Les paroles se modifient note par note dans le piano roll (`-` pour les liaisons), et les voix de référence vivent dans une **bibliothèque de voix** partagée entre les projets — changer de timbre revient à en choisir une autre
- **Échange de voix** — reprenez une voix existante, conservez sa mélodie, ses paroles et son timing, et remplacez uniquement le timbre par une voix de référence (SoulX-Singer-SVC, zero-shot). Le résultat apparaît sur une nouvelle piste sous la source et se défait d'un seul annuler ; séparer d'abord la voix d'un mixage complet évite que l'accompagnement ne perturbe le suivi de hauteur
- **Hors ligne par conception** — aucun compte, aucune clé API, aucun envoi

<p align="center">
  <img src="./assets/screenshots/07-ai-panel.png" width="100%" alt="Le panneau IA filtré sur Convertir : séparation de stems, réduction de bruit de la voix, audio vers MIDI et détection de tempo, chacun avec son modèle et son bouton d'exécution">
</p>

### Serveur MCP — Laissez un agent IA travailler dans votre session

Basculez un interrupteur et votre projet devient quelque chose qu'un assistant IA peut manipuler. Le serveur **MCP (Model Context Protocol)** intégré expose la session via HTTP, lié à `127.0.0.1` — connectez votre agent, confiez-lui une tâche et regardez-le ajouter des clips, corriger des notes ou modifier des niveaux pendant que l'interface se met à jour en temps réel. L'outil en ligne de commande `aidaw` est un client léger de ce même serveur.

- **MCP via HTTP** — un point de terminaison limité à la boucle locale, auquel votre agent se connecte
- **Modifications en direct** — les actions de l'agent apparaissent immédiatement comme des changements de projet ordinaires
- **Agent musical dans l'application** — le bouton **Écrire avec l'IA** ouvre un panneau de discussion qui compose avec les mêmes outils ; pointez-le vers tout modèle compatible OpenAI dans Paramètres → Génération de texte
- **CLI `aidaw`** — installable dans votre `PATH` depuis les Paramètres

<p align="center">
  <img src="./assets/screenshots/08-settings-ai-mcp.png" width="100%" alt="Paramètres → IA / MCP : sélection du backend d'exécution et l'interrupteur du serveur MCP, le port, l'état et le point de terminaison">
</p>

## Installation

Des binaires prêts à l'emploi seront publiés sur la page [GitHub Releases](https://github.com/heshengtao/ai-daw/releases). En attendant, effectuez une compilation depuis les sources.

### Prérequis

- Rust (stable) avec Cargo
- Node.js ≥ 18 et [pnpm](https://pnpm.io/)
- les prérequis système de Tauri v2 — voir <https://tauri.app/start/prerequisites/>

### Compiler depuis les sources

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw

pnpm install
pnpm tauri dev        # run the app in development
pnpm tauri build      # produce a release build
```

Autres commandes utiles :

```bash
pnpm build            # type-check the frontend and build it
pnpm build:aidaw      # build the aidaw CLI (see tools/build-aidaw.sh)
```

### Plateformes prises en charge

| Plateforme | État |
| --- | --- |
| Windows (x86_64) | Pris en charge |
| macOS (Apple Silicon) | Pris en charge (voir la note « application endommagée » ci-dessous) |
| Linux (x86_64 / arm64) | Pris en charge |

> **macOS affiche « Application endommagée » ?** L'application est signée ad hoc (non notariée), macOS
> met donc les téléchargements en quarantaine. Exécutez ceci dans le Terminal, puis ouvrez l'application normalement :
>
> ```bash
> xattr -cr "/Applications/AI DAW.app"
> ```

> **Les Mac Intel ne sont pas encore pris en charge.** ONNX Runtime ne publie aucun binaire précompilé pour
> `x86_64-apple-darwin`, les modèles IA locaux ne peuvent donc pas être liés pour macOS Intel.

## Accès navigateur et mode sans interface

AI DAW peut fonctionner **sans fenêtre** et servir la même interface à n'importe quel navigateur de votre réseau. L'application de bureau et le client navigateur pilotent la *même* session — même projet, même historique d'annulation, même moteur audio.

### Docker (image prête à l'emploi)

```bash
docker run -d --name ai-daw -p 28430:28430 -v ai-daw-data:/data ailm32442/ai-daw
```

Les images sont publiées sur les tags de version `v*`, et `latest` pointe vers la version la plus récente ; les commits ordinaires sur main ne reconstruisent plus l'image. Pour tester le `Dockerfile`, lancez manuellement le workflow `Docker (ai-daw)` depuis Actions.

Ouvrez <http://localhost:28430> et commencez à travailler — **aucune connexion, aucun jeton, aucune vérification de cookie**. La version sans interface est destinée aux machines dépourvues d'écran ; le contrôle d'accès relève de votre déploiement (LAN, réseau de conteneurs ou proxy inverse en amont).

Les conteneurs n'ont généralement **pas de carte son**. Dans ce cas, le moteur audio bascule sur une **horloge virtuelle** : le rendu continue de s'exécuter en temps réel, la tête de lecture, la boucle et les vumètres se comportent normalement, et le PCM sort par `/audio` — **la page web devient la carte son**. Une pastille « Horloge virtuelle » dans la barre d'état vous l'indique (cliquez dessus pour réessayer le périphérique). Pour forcer ce mode sur une machine dotée d'audio, définissez `AIDAW_AUDIO=virtual`.

**Les éditeurs de plugins fonctionnent aussi.** Le conteneur n'a pas de système de fenêtres, mais l'image embarque Xvfb : un plugin VST3 / CLAP dessine quand même son propre éditeur sur un écran virtuel, le démon encode cet écran sous forme de flux d'images, et la page le rend dans un canvas tout en renvoyant la souris et le clavier. L'éditeur devient donc une fenêtre dans la page — le même éditeur que dans une fenêtre native sur le bureau. (Dans la version de bureau, « Ouvrir l'éditeur » ouvre toujours la vraie fenêtre native : meilleure fidélité, latence plus faible.) Définissez `AIDAW_GUI_STREAM=0` pour éviter l'écran virtuel et économiser de la mémoire — vous pouvez toujours éditer les paramètres, simplement sans voir l'éditeur. L'image fait désormais environ **585 Mo** (contre ~286 Mo) : le surplus correspond à Xvfb et à l'OpenGL logiciel de Mesa, dont certains éditeurs de plugins ont besoin — sans `libgl1-mesa-dri`, ils s'affichent en noir. Si aucun de vos plugins n'utilise GL, supprimez les lignes `libgl1` / `libglx-mesa0` et récupérez ~150 Mo.


### Docker (compilation depuis les sources)

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw
docker build -t ai-daw .
docker run -d -p 28430:28430 -v ai-daw-data:/data ai-daw
```

### Binaire sans interface

```bash
npm run build                                                # produces dist/
cargo run -p ai-daw-daemon --no-default-features --bin aidawd -- \
  --data-dir ~/.ai-daw --port 28430 --web-dir dist
```

### Variables d'environnement

| Variable | Valeur par défaut | Rôle |
| --- | --- | --- |
| `AIDAW_DATA_DIR` | `/data` dans l'image | sessions, préréglages, modèles |
| `AIDAW_WEB_DIR` | `/web` dans l'image | frontend compilé à servir |
| `AIDAW_NO_AUDIO` | non défini | ne pas ouvrir de périphérique audio (conteneurs sans ALSA) |
| `AIDAW_AUDIO` | non défini | `virtual` = ne jamais toucher à une carte son, utiliser uniquement l'horloge virtuelle |
| `AIDAW_MODELS_EXTRA` | `/opt/models` dans l'image | répertoires supplémentaires de recherche de modèles (séparés par `:`, classés après le répertoire de téléchargement) |
| `AIDAW_ORT_BUNDLED` | `/opt/ort` dans l'image | répertoire ONNX Runtime embarqué |
| `AIDAW_ARTICULATION_MAPS` | `/opt/articulation-maps` dans l'image | bibliothèque d'articulations embarquée |
| `AIDAW_MCP_URL` | détecté automatiquement | adresse où le CLI `aidaw` cherche l'application en cours d'exécution |


> La version sans interface démarre aussi le serveur MCP intégré (configuration dans `<data dir>/mcp.json`, désactivé par défaut),
> pour qu'une IA puisse piloter le même projet via MCP à l'intérieur du conteneur.

### API HTTP

| Méthode | Chemin | Rôle |
| --- | --- | --- |
| `POST` | `/rpc/{command}` | toutes les commandes, avec les mêmes noms et arguments que l'IPC de l'application (`GET /commands` liste ce que sert cette version) |
| `GET` | `/events` | Server-Sent Events : modifications du moteur et état du transport |
| `GET` | `/audio` | WebSocket : PCM en direct (48 kHz stéréo float32, blocs d'environ 10 ms) |
| `GET` | `/health` | test de vie + disponibilité d'un périphérique audio |
| `GET` | `/commands` | tous les noms de commandes exposés par le serveur |

## Architecture

```
src-tauri/          Tauri shell: windows, native plugin editors, auto-updater
crates/daemon/      the whole backend: project engine, audio host, MCP, HTTP
crates/engine/      DSP + scheduling + rendering (no Tauri dependency)
crates/ipc/         the command/event schema shared by app, CLI and MCP
crates/extensions/  app-extension host: npm install, manifests, QuickJS tools, client panes
src/                Vue 3 frontend — identical for desktop and browser
```

Le frontend ne dialogue jamais directement avec Rust : `src/api/ipc.ts` choisit un transport (`Tauri IPC` ou `HTTP + SSE + WebSocket`), si bien que chaque écran fonctionne dans les deux environnements. C'est pourquoi la version navigateur n'est pas un portage — c'est la même application qui parle au même backend à travers le réseau.

## FAQ

### AI DAW est-il gratuit ?

Oui. AI DAW est open source sous licence Apache 2.0. Il n'y a ni offre payante ni fonctionnalité verrouillée.

### Fonctionne-t-il hors ligne ?

Oui. L'application et tous les modèles IA s'exécutent localement. Les seuls accès réseau servent à télécharger des paquets de plugins et des modèles IA (que vous déclenchez), à récupérer les index de plugins et d'extensions, et à appeler un service musical cloud si vous en configurez un.

### Faut-il un GPU ?

Non. Les modèles IA s'exécutent sur le CPU par défaut. macOS peut utiliser CoreML et Windows peut utiliser DirectML lorsque le matériel le permet ; les deux sont optionnels.

### Quels modèles IA sont fournis avec l'application ?

Deux petits le sont : **Basic Pitch** (audio vers MIDI) et **LocalVQE** (nettoyage de voix), qui fonctionnent donc dès l'installation. Les modèles plus volumineux — **Beat This!** (détection de tempo) et **HTDemucs** (séparation de stems, 4 ou 6 stems) — sont téléchargés à la demande depuis le gestionnaire de modèles de l'application lors de leur première utilisation, et stockés en dehors de l'arborescence des sources.

Aucun poids de modèle ne fait jamais partie du dépôt ni des installateurs, et chaque modèle conserve la licence de son éditeur : voir [MODELS.md](MODELS.md). Cela inclut les modèles optionnels sous CC BY-NC 4.0 (non commercial) — **YuE2** pour la génération de morceaux et **SheetSage2** pour les partitions de reprises.

### Quels formats de plugins sont pris en charge ?

VST3 et CLAP sous Windows, macOS et Linux ; **Audio Units (AU / AUv3)** également sous macOS.

### Quelles langues l'interface prend-elle en charge ?

Chinois simplifié, chinois traditionnel, anglais, japonais, coréen, espagnol, français, allemand, russe et arabe. Changez à tout moment dans **Paramètres → Général → Langue** ; votre choix est mémorisé et, tant que vous ne le modifiez pas, l'interface suit la langue de votre système. Les dictionnaires se trouvent dans `src/i18n/`, et toute chaîne absente d'une traduction revient à l'anglais. Le texte arabe est entièrement pris en charge — la timeline conserve sa disposition de gauche à droite, car l'axe temporel d'un DAW se lit de gauche à droite. Le README est traduit dans ces mêmes dix langues.

### Faut-il que l'application soit ouverte pour qu'un agent la pilote ?

Oui. Le Skill et le CLI `aidaw` dialoguent avec une instance d'AI DAW en cours d'exécution via MCP : lancez d'abord l'application (`aidaw status` vous indiquera si elle est active).

## Nous soutenir

### Mettez-nous une étoile !
⭐ Votre soutien est le moteur qui nous fait avancer !

### Les pourboires sont bienvenus !
<div align="center" style="display: flex; gap: 20px; justify-content: center; flex-wrap: wrap;">

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/agentparty)
[![爱发电](https://img.shields.io/badge/爱发电-支持我们-946ce6?style=for-the-badge&logo=affine&logoColor=white)](https://afdian.com/a/agentparty)

</div>

### Suivez-nous
<div align="center">
  <a href="https://space.bilibili.com/26978344">
    <img src="screenshots/B.png" width="100" height="100" style="border-radius: 80%; overflow: hidden;" alt="bilibili"/>
  </a>
  <a href="https://www.youtube.com/@agentParty">
    <img src="screenshots/YT.png" width="100" height="100" style="border-radius: 80%; overflow: hidden;" alt="youtube"/>
  </a>
</div>

### Rejoignez la communauté
Si vous avez des questions ou rencontrez des problèmes avec le projet, vous êtes les bienvenus dans notre communauté.

1. Groupe QQ : `931057213` (complet) / `902882342` (groupe 2)

2. Discord : [Lien Discord](https://discord.gg/f2dsAKKr2V)

## Contributeurs

<a href="https://github.com/heshengtao/ai-daw/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=heshengtao/ai-daw" alt="Contributeurs de heshengtao/ai-daw" />
</a>

## Licence

Le **code** de l'application est sous Apache-2.0 — voir [LICENSE](LICENSE).

Les **poids** des modèles forment une couche distincte : ils ne font jamais partie de ce dépôt ni des
installateurs, ils conservent la licence que leur éditeur leur a donnée, et l'application les télécharge
à la demande (en vérifiant le `sha256` de chaque fichier téléchargé). Voir **[MODELS.md](MODELS.md)** pour le tableau
complet des modèles, des sources et des licences.

Certains modèles optionnels sont sous **CC BY-NC 4.0 (non commercial)** — YuE2 pour la génération de morceaux et
SheetSage2 pour les partitions de reprises. Ce sont des téléchargements optionnels : n'utilisez pas ces poids pour un
usage commercial, sauf si vous détenez une licence commerciale de leurs éditeurs. L'audio et le MIDI générés vous
appartiennent, dans les conditions des licences amont et sous réserve des droits que vous détenez sur l'audio d'entrée.

## Licences tierces

Ce projet s'appuie sur des logiciels open source tiers. Les fichiers adaptés d'autres projets, ainsi que les dépendances dont la licence ne fait pas partie de l'ensemble permissif par défaut, sont listés dans [NOTICE](NOTICE).
