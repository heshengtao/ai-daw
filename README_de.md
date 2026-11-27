<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW — eine lokal arbeitende, KI-native digitale Audio-Workstation">
</p>

<p align="center">
  <a href="https://github.com/heshengtao/ai-daw/releases"><img src="https://img.shields.io/github/v/release/heshengtao/ai-daw?style=flat-square&color=1A6CFF" alt="GitHub-Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="Lizenz: Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux%20/%20Web-1A6CFF?style=flat-square" alt="Plattformen: Windows, macOS, Linux, Web">
  <a href="https://hub.docker.com/r/ailm32442/ai-daw"><img src="https://img.shields.io/docker/pulls/ailm32442/ai-daw?style=flat-square&color=1A6CFF" alt="Docker-Pulls"></a>
</p>

<p align="center">
  <a href="README_zh.md">简体中文</a> · <a href="README_zh-TW.md">繁體中文</a> · <a href="README.md">English</a> · <a href="README_ja.md">日本語</a> ·
  <a href="README_ko.md">한국어</a> · <a href="README_es.md">Español</a> · <a href="README_fr.md">Français</a> · <a href="README_de.md"><b>Deutsch</b></a> ·
  <a href="README_ru.md">Русский</a> · <a href="README_ar.md">العربية</a>
</p>

<p align="center">
  <a href="https://hub.docker.com/r/ailm32442/ai-daw"><b>Docker Hub</b></a> ·
  <a href="https://github.com/heshengtao/ai-daw/releases">Herunterladen</a>
</p>

## Was ist AI DAW?

**AI DAW ist ein kleines, KI-natives Musikstudio für Ihren Desktop** — schreiben, arrangieren, mischen und mastern Sie in einer einzigen schlanken App, und lassen Sie einen KI-Agenten in derselben Session an Ihrer Seite arbeiten.

Es ist eine echte DAW: eine Mehrspur-Timeline, eine Klavierrolle, ein Mixer mit Sends und Bussen, Automatisierungsspuren, ein eingebauter Synth und Hosting für VST3-/CLAP-/AU-Plugins. Sie ist außerdem darauf ausgelegt, **von einem Modell bedient zu werden** — ein mitgelieferter Skill bringt jedem AI-Harness bei, die laufende App über MCP zu steuern, sodass ein Agent Spuren hinzufügen, Noten schreiben, einen Mix ausbalancieren und das Ergebnis analysieren kann, während Sie es in der Oberfläche mitverfolgen.

Alles läuft auf Ihrem eigenen Rechner. Kein Konto, kein API-Schlüssel, kein Audio verlässt Ihren Computer.

<img src="./assets/screenshots/01-arrange.png" width="100%" alt="AI DAW — die Arrangement-Ansicht: sechs Spuren mit Clips auf der Timeline, rechts der Inspector mit einer EQ- und Reverb-Kette und unten eine Automatisierungsspur">

## Warum das wichtig ist

Modelle sind gut im Schreiben von Code geworden, weil sie ihn über ein Coding-Agent-Framework **ausführen** konnten; Modelle sind gut in der 3D-Modellierung geworden, weil sie echte Modellierungssoftware über Blender MCP steuern konnten. In beiden Fällen bekommt das Modell ein echtes Werkzeug, ein echtes Ergebnis und eine echte Rückkopplungsschleife — und genau das macht schnelle Iteration möglich.

Der Musik hat diese Schleife weitgehend gefehlt. Ein Modell kann Audio erzeugen, aber es kann nicht *an einer Session arbeiten* wie ein Mensch: kein Arrangement zum Bearbeiten, kein Mixer zum Nachführen, keine Undo-Historie, keine Möglichkeit festzustellen, dass die letzte Änderung alles schlechter gemacht hat.

**AI DAW ist ein kleiner, gezielter Versuch, diese Lücke zu schließen.** Es ist eine echte DAW, die ein KI-Agent über einen Skill und MCP bedienen kann, was sie nützlich macht für:

- **Zu testen, was ein Modell wirklich über Songwriting weiß** — nicht, ob sein Audio plausibel klingt, sondern ob es ein Arrangement aufbauen, Parts auswählen und einen Mix in Ordnung bringen kann.
- **Echte Songwriting-Traces zu sammeln** — jede Bearbeitung durch den Agenten landet in einem normalen Projekt mit einer normalen Undo-Historie, sodass eine Session zu Daten darüber wird, wie ein Modell über Musik nachdenkt.
- **Einen Schritt hin zu Modellen, die Musik so schreiben können, wie sie Code schreiben** — mit einem Werkzeug, das zurückspricht.

## Leichtgewicht als Konstruktionsprinzip

Klein zu sein ist ein Feature, kein Kompromiss.

- **Ein schlanker Fußabdruck** — kein Electron, keine Cloud, kein mitgelieferter Modell-Zoo. Die macOS-App ist rund 38 MB groß — zum Vergleich: gängige DAWs wie Ableton Live, Logic Pro und FL Studio belegen bei der Installation typischerweise 1–3 GB, womit AI DAW **über 95 % kleiner** ist.
- **Schneller Start** — beim Start wird nichts Schwergewichtiges geladen; KI-Modelle werden erst geholt, wenn Sie sie verwenden.
- **Große Modelle auf Abruf, nie im Installer** — schwere Arbeit wie die Musikgenerierung ist standardmäßig nicht integriert. **Gesangsstimme** (SoulX-Singer, 1,4 GB) und **Stimmtausch** (SoulX-Singer-SVC, 2,4 GB) sind optionale Modelle unter Manager → Modelle: Laden Sie sie herunter, wenn Sie sie brauchen — bis dahin belegen sie keinen Speicherplatz.
- **Zum Erweitern gebaut** — **App-Erweiterungen** (npm-Pakete) ergänzen bereits Agenten-Werkzeuge und Seitenleisten-Panels; Schnittstellen, um externe KI-Modelle in den Workflow einzubinden, stehen auf der Roadmap.

## Aus jedem AI-Harness bedienen

AI DAW liefert einen **Skill** (`skills/aidaw`) mit, der einem KI-Agenten beibringt, die laufende App zu bedienen — das Projekt, die Spuren, Clips und Noten lesen und schreiben; einen Stil analysieren, Musiktheorie und Drum-Patterns nachschlagen und ein Genre-Rezept in MIDI verwandeln; mischen (Gain, Panorama, Sends, Automatisierung); Transport und Aufnahme; Loudness- und Spektrumanalyse; WAV-Export; Plugin-Scan. Er funktioniert über den eingebauten **MCP**-Server, sodass ihn jedes Harness übernehmen kann, das Agent Skills unterstützt.

Die App bringt außerdem 18 gesampelte Instrumente mit — **Konzertflügel, Nylon- und Clean-E-Gitarren, Fingerbass, ein akustisches Drumkit, ein Streichensemble, Slow Strings, Solo-Violine/Viola/Cello/Kontrabass, Pizzicato-Streicher, Trompete, Waldhörner sowie Synth-Streicher/Brass 1 & 2** (SF3/SF2, insgesamt ca. 13,5 MB) — sofort spielbereit.

**Option 1 · Aus der Kommandozeile installieren**

```bash
npx skills add heshengtao/ai-daw
```

**Option 2 · Ihren Agenten bitten, ihn zu installieren**

```
Install this Skill: https://github.com/heshengtao/ai-daw
```

Jede Bearbeitung eines Agenten läuft durch dieselbe Befehlspipeline wie die UI: Sie landet in der Undo-Historie, erscheint in der Oberfläche und wird dem Agenten zugeschrieben — Sie können also jederzeit sehen, was er getan hat, und es rückgängig machen. Lieber in der App bleiben? **Mit KI schreiben** öffnet dieselbe Schleife als Chat-Panel (Einstellungen → Texterzeugung).

## Funktionen

### Inspire — Eine Zeile rein, ein fertiger Take raus

Die Ansicht ganz links in der oberen Leiste ist **Inspire**: ein ganzseitiger Arbeitsplatz „schreiben → generieren → anhören“. Links schreiben, rechts hören. Keine Likes, keine Shares, keine Upgrade-Schranken — nur Ihre Beschreibung, Ihr Songtext und die Wiedergabeleiste.

- **Ein Dropdown, zwei Engines** — lokale YuE2-/ACE-Step-Gewichte und jeder konfigurierte Cloud-Dienst (Suno / Mureka / Lyria …) teilen sich eine einzige Liste; einen Anbieter hinzuzufügen heißt, eine Stelle zu ändern
- **Einfach / Erweitert** — Einfach nimmt eine Zeile Beschreibung; Erweitert lässt Sie einen Songtext mit `[Verse]`-/`[Chorus]`-/`[Bridge]`-/`[Outro]`-Tags schreiben, den Schalter **Instrumental** umlegen und Dauer, Schritte und Seed einstellen
- **Takes im Stapel** — fordern Sie mit einem Klick bis zu 8 Takes an; sie rendern nacheinander, jeder mit einem frischen Seed, und erscheinen in der Liste, sobald sie fertig sind
- **Einen Take verlängern** — jedes lokale Rendering behält einen semantischen Code, sodass **Verlängern** den nächsten Abschnitt dort ansetzt, wo dieser Take aufgehört hat — das Modell bekommt die Struktur, nicht nur das Audio
- **Es holt sich, was es braucht** — fehlende Gewichte, eine fehlende Engine oder eine gestoppte Engine werden der Reihe nach abgearbeitet, wenn Sie auf Erzeugen drücken
- **Takes in einer Spalte** — nach Tagen gruppiert; jeder Take bekommt ein **generiertes Cover** in den Spur-Farbtönen des Projekts, eine immer sichtbare Abspielschaltfläche und eine eigene Wiedergabeleiste: Klicken zum Anhören, Ziehen zum Scrubben. Die fokussierte Karte ist auch die, die die Transporttaste in der unteren Leiste steuert
- **Bleibt bei Ihnen** — der Erzeugungsverlauf liegt auf Ihrem Rechner; ihn zu löschen berührt das Projekt nie, und ein Take gelangt nur dann in die Session (＋) oder auf die Festplatte (Speichern), wenn Sie es verlangen

<p align="center">
  <img src="./assets/screenshots/02-inspire.png" width="100%" alt="Die Inspire-Seite: links der Composer im Modus Erweitert mit Stilbeschreibung, Songtext und Takes, rechts der Erzeugungsverlauf mit generierten Covern und Wiedergabeleisten">
</p>

### Arrangieren & Bearbeiten — Auf einer echten Timeline komponieren

Legen Sie einen Song so an, wie Sie ihn hören. Ziehen Sie Clips über Spuren, teilen Sie sie dort, wo die Musik wechselt, und lassen Sie Lineal und Raster einem Tempo und Takt folgen, die Sie an jeder Stelle des Songs ändern können. MIDI-Clips zeichnen ihre eigenen Noten, Audioclips ihre eigene Wellenform — so erfassen Sie das Arrangement auf einen Blick.

- **Mehrspuriges Arrangieren** — MIDI- und Audioclips nebeneinander, Drag & Drop über Spuren hinweg, Gummiband-Auswahl
- **Eine Klavierrolle, die sich gut liest** — Clip-Vorschauen zeigen Noten; der Editor bietet Velocity, Zoom und eine virtuelle Tastatur
- **Notation und Tabulatur** — die Partitur-Spur zeichnet aus denselben Noten ein Notensystem oder eine Gitarren-Tabulatur, und beim Export kommt MusicXML heraus
- **Dynamisches Tempo & Metrum** — ändern Sie Tempo oder Taktart an jeder beliebigen Stelle; das Raster folgt
- **Transport & Loop** — Wiedergabe, Stopp, Spulen, Bereich loopen, Metronom
- **Automatisierungsspuren** — zeichnen Sie Kurven für alles, was ein Plugin oder der eingebaute Synth anbietet
- **Werkzeuge am Zeiger** — ein schwebendes Dock schaltet zwischen Zeichnen / Radieren / Auswählen / Teilen / Songtext um, und ein Klick wechselt die untere Spur zwischen Automatisierung, Partitur und Tastatur
- **Ein echtes Bearbeitungs-Toolkit** — Teilen, Verschieben, Kopieren/Einfügen, Rückgängig/Wiederholen, dazu ein Spickzettel mit den Tastaturkürzeln

<p align="center">
  <img src="./assets/screenshots/03-edit.png" width="100%" alt="Der Klavierrollen-Editor: die Noten eines Melodie-Clips mit der Velocity-Spur darunter und die EQ- und Reverb-Kette der Spur im Inspector">
</p>

### Mischen & Mastern — Den Track fertigstellen

Bringen Sie den Song ins Gleichgewicht und machen Sie ihn dann laut. Kanalzüge bieten Gain, Panorama, Sends, Busse, Stumm/Solo und Pegelanzeige; eine Mastering-Kette auf dem Master-Bus kümmert sich um Loudness, Peaks und Stereo-Breite, sodass Sie etwas Veröffentlichungsfertiges rendern können, ohne die App zu verlassen.

- **Mixer** — Gain, Panorama, Sends, Busse, Stumm/Solo und Pegelanzeige
- **Eingebaute Effekte** — Dynamik, EQ und zeitbasierte Effekte, bereit zum Ablegen auf jeder Spur
- **Mastering-Kette** — Loudness-Normalisierung, True-Peak-Limiting und Stereo-Breite
- **Eingebauter Synth** — Oszillatoren, Hüllkurven und ein Filter mit Live-Oszilloskop
- **Export** — verlustfreies WAV oder LAME-kodiertes MP3 (Abtastrate und Bitrate nach Wahl), eine Standard-MIDI-Datei oder eine MusicXML-Partitur

<p align="center">
  <img src="./assets/screenshots/04-mix.png" width="100%" alt="Die Mix-Ansicht: Kanalzüge für den Master-Bus, fünf Instrumentenspuren und einen Reverb-Bus, mit Fadern, Panorama, Sends, Inserts und Live-Pegeln">
</p>

### Plugins — Eigene Instrumente und Effekte mitbringen

AI DAW hostet **VST3**- und **CLAP**-Plugins überall, zusätzlich **Audio Units (AU / AUv3)** unter macOS, sodass die Werkzeuge, die Sie bereits besitzen, mitkommen. Ein Plugin-Manager durchsucht die offene Registry, filtert nach Typ und Betriebssystem-Kompatibilität und installiert, aktualisiert oder entfernt Plugins mit einem Klick — oder Sie verweisen ihn auf etwas, das Sie bereits auf der Festplatte haben.

- **VST3- / CLAP- / AU-Hosting** — laden Sie Instrumente und Effekte von Drittanbietern auf jede Spur (AU und AUv3 unter macOS)
- **Plugin-Manager** — die Registry durchsuchen, nach Instrument/Effekt und Plattform filtern, installieren / aktualisieren / deinstallieren
- **Lokaler Import** — ein Plugin übernehmen, das Sie anderswo installiert haben
- **Editoren & Parameter** — die eigene Oberfläche eines Plugins öffnen oder seine Parameter aus dem Inspector und den Automatisierungsspuren steuern; AU-Plugins ohne eigene Ansicht erhalten ein generisches Parameter-Panel
- **SoundFont-Bibliotheken** — importieren Sie Ihre eigenen `.sf2` / `.sf3` (eine Bank entfaltet sich zu Dutzenden von Presets) oder installieren Sie eine offene Bank wie MuseScore General, FluidR3 oder FreePats aus dem integrierten SoundFont-Index
- **Ein Download-Manager** — Plugins, SoundFonts, KI-Modelle und Runtimes teilen sich einen einzigen Manager mit Live-Fortschritt, Abbrechen und einem Installationsstatus pro Eintrag

<p align="center">
  <img src="./assets/screenshots/05-plugins.png" width="100%" alt="Das Plugin-Panel: die SoundFont-Bibliothek plus integrierte Instrumente nach Familien gruppiert, mit Filtern für integriert / extern">
</p>

### App-Erweiterungen — Die App selbst erweitern

Audio-Plugins erweitern den Klang; **App-Erweiterungen** erweitern die Anwendung. Sie werden als npm-Pakete ausgeliefert und können **MCP-Werkzeuge**, die ein Agent aufrufen kann, sowie **Panels** hinzufügen, die in der rechten Seitenleiste gerendert werden — ein Workflow, der Ihnen fehlt, lässt sich also ergänzen, ohne die App zu forken.

- **npm-Verteilung** — Installation aus einem Index (`index.json`) oder direkt aus einer Registry; `engines.aidaw` sagt, welche App-Versionen eine Erweiterung unterstützt, und Mirrors decken Offline- oder Firewall-Netze ab
- **Entdecken und Installiert** — das Panel listet alles, was Ihre Indizes kennen (Version, Autor, Installieren / Aktualisieren), neben allem, was läuft (Status, Werkzeuge, Panes, Aktivieren / Neu laden / Deinstallieren)
- **Zwei Seiten einer Erweiterung** — serverseitige Werkzeuge laufen im QuickJS-Host des Daemons und erreichen die Session über dieselben Befehle wie die UI; clientseitige Panes sind ESM-Module, die mit Hot Reload in die Webview geladen werden
- **Auch lokale Ordner** — „Ordner importieren“ installiert eine Erweiterung, die Sie selbst entwickeln; `examples/extensions/hello` ist die minimale Referenz (ein Werkzeug, ein Pane)
- **Nicht die Audiokette** — für VST3 / CLAP / AU siehe **Plugins** oben; das hier ist die App, nicht der Mixer

<p align="center">
  <img src="./assets/screenshots/06-extensions.png" width="100%" alt="Das Panel „App-Erweiterungen“: die Liste „Entdecken“ mit Versionen, Autoren und Installationsschaltflächen, der Index-Status-Chip und die Tabs „Installiert“ / „Entdecken“">
</p>

### Lokale KI — Audio in Material verwandeln, mit dem Sie arbeiten können

Die KI-Funktionen sind für echte Sessions gebaut, nicht für Demos. Richten Sie sie auf eine Spur, und sie geben Ihnen etwas zurück, das Sie weiter bearbeiten können: MIDI-Noten, die Sie verschieben können, einen Gesangstake mit weniger Raumhall, Stems, die Sie neu ausbalancieren können. Alles läuft auf Ihrer eigenen Hardware — standardmäßig auf der CPU, mit optionaler CoreML- (macOS) und DirectML-Beschleunigung (Windows). Jeder Job liegt in einem **KI-Panel**, gefiltert nach **Alle / Erzeugen / Konvertieren**, und jede Zeile sagt Ihnen, ob das jeweilige Modell bereits installiert ist.

- **Tempo-Erkennung** — Beat This! (ISMIR 2024) ermittelt in einem Durchgang ein globales BPM und eine Taktart
- **Audio-zu-MIDI** — Basic Pitch verwandelt eine aufgenommene Performance in bearbeitbare Noten; wechseln Sie das Modell zu **MuScriptor** (über die audio.cpp-Engine) für Multi-Instrument-Transkription, die eine MIDI-Spur pro Instrument anlegt. 393 MB–5,1 GB, bei Bedarf heruntergeladen (CC BY-NC 4.0 — nicht kommerziell)
- **Gesangsbereinigung** — LocalVQE reduziert Rauschen und Hall bei Gesangstakes
- **Stem-Trennung** — HTDemucs v4 teilt eine Spur in 4 oder 6 Stems, bereit zum Remixen
- **Gesangsstimme** — eine MIDI-Melodie, ein Songtext und eine Referenzstimme werden zu einem bearbeitbaren Gesangstake (Zero-Shot-Timbre-Klonen mit SoulX-Singer). Der Songtext lässt sich pro Note in der Klavierrolle bearbeiten (`-` für Bindebögen), und Referenzstimmen liegen in einer **Stimmbibliothek**, die projektübergreifend geteilt wird — die Klangfarbe zu ändern heißt einfach, eine andere auszuwählen
- **Stimmtausch** — nehmen Sie einen vorhandenen Gesang, behalten Sie Melodie/Songtext/Timing bei und ersetzen Sie nur die Klangfarbe durch eine Referenzstimme (SoulX-Singer-SVC, Zero-Shot). Das Ergebnis landet auf einer neuen Spur unter der Quelle und ist nur ein einziges Undo entfernt; trennen Sie den Gesang zuerst aus einem kompletten Mix, bleibt die Begleitung aus dem Pitch-Tracker heraus
- **Offline als Prinzip** — kein Konto, kein API-Schlüssel, keine Uploads

<p align="center">
  <img src="./assets/screenshots/07-ai-panel.png" width="100%" alt="Das KI-Panel, gefiltert auf Konvertieren: Stem-Trennung, Gesangs-Denoise, Audio-zu-MIDI und Tempo-Erkennung, jeweils mit Modell und Startschaltfläche">
</p>

### MCP-Server — Lassen Sie einen KI-Agenten in Ihrer Session arbeiten

Legen Sie einen Schalter um, und Ihr Projekt wird zu etwas, das ein KI-Assistent bedienen kann. Der eingebaute Server für das **MCP (Model Context Protocol)** stellt die Session über HTTP bereit, gebunden an `127.0.0.1` — verbinden Sie Ihren Agenten, geben Sie ihm eine Aufgabe und sehen Sie zu, wie er Clips hinzufügt, Noten korrigiert oder Pegel ändert, während die UI in Echtzeit mitläuft. Das Kommandozeilenwerkzeug `aidaw` ist ein schlanker Client für denselben Server.

- **MCP über HTTP** — ein nur über Loopback erreichbarer Endpunkt, mit dem sich Ihr Agent verbindet
- **Live-Bearbeitung** — Agentenaktionen erscheinen sofort als gewöhnliche Projektänderungen
- **Song-Agent in der App** — die Schaltfläche **Mit KI schreiben** öffnet ein Chat-Panel, das gegen dieselben Werkzeuge komponiert; richten Sie es unter Einstellungen → Texterzeugung auf ein beliebiges OpenAI-kompatibles Modell
- **`aidaw`-CLI** — aus den Einstellungen auf Ihren `PATH` installierbar

<p align="center">
  <img src="./assets/screenshots/08-settings-ai-mcp.png" width="100%" alt="Einstellungen → KI / MCP: Auswahl des Ausführungs-Backends sowie Schalter, Port, Status und Endpunkt des MCP-Servers">
</p>

## Installation

Vorgefertigte Binärpakete werden auf der Seite [GitHub Releases](https://github.com/heshengtao/ai-daw/releases) veröffentlicht. Bis dahin bauen Sie aus dem Quellcode.

### Voraussetzungen

- Rust (stable) mit Cargo
- Node.js ≥ 18 und [pnpm](https://pnpm.io/)
- Systemvoraussetzungen für Tauri v2 — siehe <https://tauri.app/start/prerequisites/>

### Aus dem Quellcode bauen

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw

pnpm install
pnpm tauri dev        # die App im Entwicklungsmodus starten
pnpm tauri build      # einen Release-Build erzeugen
```

Weitere nützliche Befehle:

```bash
pnpm build            # das Frontend typprüfen und bauen
pnpm build:aidaw      # die aidaw-CLI bauen (siehe tools/build-aidaw.sh)
```

### Plattform-Unterstützung

| Plattform | Status |
| --- | --- |
| Windows (x86_64) | Unterstützt |
| macOS (Apple Silicon) | Unterstützt (siehe Hinweis „App ist beschädigt“ unten) |
| Linux (x86_64 / arm64) | Unterstützt |

> **macOS meldet „App ist beschädigt“?** Die App ist ad-hoc signiert (nicht notarisiert), daher legt macOS
> heruntergeladene Dateien unter Quarantäne. Führen Sie dies im Terminal aus und öffnen Sie die App dann normal:
>
> ```bash
> xattr -cr "/Applications/AI DAW.app"
> ```

> **Intel-Macs werden noch nicht unterstützt.** ONNX Runtime veröffentlicht keine vorgebauten
> Binärpakete für `x86_64-apple-darwin`, daher lassen sich die lokalen KI-Modelle für Intel-macOS nicht linken.

## Browser-Zugriff & Headless

AI DAW kann **ohne Fenster** laufen und dieselbe Oberfläche jedem Browser in Ihrem Netzwerk bereitstellen. Die Desktop-App und der Browser-Client steuern *dieselbe* Session — dasselbe Projekt, dieselbe Undo-Historie, dieselbe Audio-Engine.

### Docker (vorgefertigtes Image)

```bash
docker run -d --name ai-daw -p 28430:28430 -v ai-daw-data:/data ailm32442/ai-daw
```

Images werden bei `v*`-Release-Tags veröffentlicht, und `latest` zeigt auf das jüngste
Release; gewöhnliche Commits auf main bauen das Image nicht mehr neu. Um das
`Dockerfile` zu testen, starten Sie den Workflow `Docker (ai-daw)` manuell über Actions.

Öffnen Sie <http://localhost:28430> und legen Sie los — **kein Login, kein Token, keine Cookie-Prüfung**.
Der Headless-Build ist für Rechner ohne Display gedacht; die Zugriffskontrolle ist
Sache des Deployments (LAN, Container-Netzwerk oder ein Reverse-Proxy davor).

Container haben meist **keine Soundkarte**. In diesem Fall schaltet die Audio-Engine auf eine
**virtuelle Uhr** um: Das Rendern läuft weiterhin in Echtzeit, Playhead / Loop / Pegel verhalten sich
normal, und die PCM-Daten gehen über `/audio` hinaus — **die Webseite wird zur Soundkarte**.
Ein Chip „Virtuelle Uhr“ in der Statusleiste sagt es Ihnen (ein Klick darauf versucht das Gerät erneut). Um diesen
Pfad auf einem Rechner zu erzwingen, der Audio hat, setzen Sie `AIDAW_AUDIO=virtual`.

**Plugin-Editoren funktionieren ebenfalls.** Der Container hat kein Fenstersystem, aber das Image bringt Xvfb mit:
Ein VST3-/CLAP-Plugin zeichnet seinen eigenen Editor weiterhin auf einem virtuellen Display, der Daemon kodiert
diesen Bildschirm als Frame-Stream, und die Seite rendert ihn auf ein Canvas, während Maus und
Tastatur zurückgeleitet werden. So wird der Editor zu einem Fenster in der Seite — derselbe Editor, den Sie
in einem nativen Fenster auf dem Desktop bekommen würden. (Im Desktop-Build öffnet „Editor öffnen“ weiterhin das echte native
Fenster: höhere Wiedergabetreue, geringere Latenz.) Setzen Sie `AIDAW_GUI_STREAM=0`, um das virtuelle Display
zu überspringen und Speicher zu sparen — Sie können Parameter weiterhin bearbeiten, nur den Editor nicht sehen.
Das Image ist inzwischen etwa **585 MB** groß (früher ~286 MB): Der Zuwachs sind Xvfb plus Mesa-Software-
OpenGL, das manche Plugin-Editoren brauchen — ohne `libgl1-mesa-dri` rendern sie schwarz. Wenn keines
Ihrer Plugins GL nutzt, streichen Sie die Zeilen `libgl1` / `libglx-mesa0` und gewinnen ~150 MB zurück.


### Docker (aus dem Quellcode bauen)

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw
docker build -t ai-daw .
docker run -d -p 28430:28430 -v ai-daw-data:/data ai-daw
```

### Headless-Binärdatei

```bash
npm run build                                                # erzeugt dist/
cargo run -p ai-daw-daemon --no-default-features --bin aidawd -- \
  --data-dir ~/.ai-daw --port 28430 --web-dir dist
```

### Umgebungsvariablen

| Variable | Standard | Zweck |
| --- | --- | --- |
| `AIDAW_DATA_DIR` | `/data` im Image | Sessions, Presets, Modelle |
| `AIDAW_WEB_DIR` | `/web` im Image | gebautes Frontend, das ausgeliefert wird |
| `AIDAW_NO_AUDIO` | nicht gesetzt | kein Audiogerät öffnen (Container ohne ALSA) |
| `AIDAW_AUDIO` | nicht gesetzt | `virtual` = nie eine Soundkarte berühren, nur die virtuelle Uhr verwenden |
| `AIDAW_MODELS_EXTRA` | `/opt/models` im Image | zusätzliche Modell-Suchverzeichnisse (`:`-getrennt, nach dem Download-Verzeichnis eingereiht) |
| `AIDAW_ORT_BUNDLED` | `/opt/ort` im Image | mitgeliefertes ONNX-Runtime-Verzeichnis |
| `AIDAW_ARTICULATION_MAPS` | `/opt/articulation-maps` im Image | mitgelieferte Bibliothek der Artikulations-Maps |
| `AIDAW_MCP_URL` | automatisch ermittelt | wo die `aidaw`-CLI nach der laufenden App sucht |


> Der Headless-Build startet außerdem den eingebetteten MCP-Server (Konfiguration unter `<data dir>/mcp.json`, standardmäßig ausgeschaltet),
> sodass eine KI dasselbe Projekt im Container über MCP steuern kann.

### HTTP-API

| Methode | Pfad | Zweck |
| --- | --- | --- |
| `POST` | `/rpc/{command}` | jeder Befehl, mit denselben Namen und Argumenten wie das IPC der App (`GET /commands` listet, was dieser Build anbietet) |
| `GET` | `/events` | Server-Sent Events: Engine-Bearbeitungen und Transportzustand |
| `GET` | `/audio` | WebSocket: Live-PCM (48 kHz Stereo float32, ~10 ms Blöcke) |
| `GET` | `/health` | Liveness + ob ein Audiogerät verfügbar ist |
| `GET` | `/commands` | jeder Befehlsname, den der Server bereitstellt |

## Architektur

```
src-tauri/          Tauri-Shell: Fenster, native Plugin-Editoren, Auto-Updater
crates/daemon/      das gesamte Backend: Projekt-Engine, Audio-Host, MCP, HTTP
crates/engine/      DSP + Scheduling + Rendering (ohne Tauri-Abhängigkeit)
crates/ipc/         das Befehls-/Event-Schema, das App, CLI und MCP teilen
crates/extensions/  Host für App-Erweiterungen: npm-Install, Manifeste, QuickJS-Werkzeuge, Client-Panes
src/                Vue-3-Frontend — identisch für Desktop und Browser
```

Das Frontend spricht nie direkt mit Rust: `src/api/ipc.ts` wählt einen Transport (`Tauri IPC`
oder `HTTP + SSE + WebSocket`), sodass jede Ansicht in beiden Shells funktioniert. Deshalb ist der Browser-
Build kein Port — es ist dieselbe App, die über das Netzwerk mit demselben Backend spricht.

## FAQ

### Ist AI DAW kostenlos?

Ja. AI DAW ist Open Source unter Apache 2.0. Es gibt keine bezahlten Stufen und keine Feature-Gates.

### Funktioniert es offline?

Ja. Die App und jedes KI-Modell laufen lokal. Der einzige Netzwerkzugriff ist das Herunterladen von Plugin-Paketen und KI-Modellen (das Sie selbst starten), das Abrufen der Plugin- und Erweiterungs-Indizes und der Aufruf eines Cloud-Musikdienstes, wenn Sie einen konfigurieren.

### Brauche ich eine GPU?

Nein. KI-Modelle laufen standardmäßig auf der CPU. macOS kann CoreML nutzen, Windows DirectML, wenn die Hardware es unterstützt; beides ist optional.

### Welche KI-Modelle werden mit der App ausgeliefert?

Zwei kleine sind dabei: **Basic Pitch** (Audio-zu-MIDI) und **LocalVQE** (Gesangsbereinigung), diese funktionieren also sofort nach der Installation. Die größeren Modelle — **Beat This!** (Tempo-Erkennung) und **HTDemucs** (Stem-Trennung, 4 oder 6 Stems) — werden beim ersten Einsatz bei Bedarf aus dem Modell-Manager in der App heruntergeladen und außerhalb des Quellbaums gespeichert.

Keine Modellgewichte sind jemals Teil des Repositories oder der Installer, und jedes Modell behält die Lizenz seines Herausgebers: siehe [MODELS.md](MODELS.md). Das schließt die optionalen CC BY-NC 4.0-Modelle (nicht kommerziell) ein — **YuE2** für die Songgenerierung und **SheetSage2** für Cover-Partituren.

### Welche Plugin-Formate werden unterstützt?

VST3 und CLAP unter Windows, macOS und Linux; **Audio Units (AU / AUv3)** zusätzlich unter macOS.

### Welche Sprachen unterstützt die Oberfläche?

Vereinfachtes Chinesisch, traditionelles Chinesisch, Englisch, Japanisch, Koreanisch, Spanisch, Französisch, Deutsch, Russisch und Arabisch. Wechseln Sie jederzeit unter **Einstellungen → Allgemein → Sprache**; die Wahl wird gemerkt und folgt bis dahin Ihrer Systemsprache. Die Wörterbücher liegen in `src/i18n/`, und jede Zeichenkette, für die eine Übersetzung fehlt, fällt auf Englisch zurück. Arabischer Text wird vollständig unterstützt — die Timeline behält ihr Links-nach-rechts-Layout, weil die Zeitachse einer DAW von links nach rechts gelesen wird. Die README ist in dieselben zehn Sprachen übersetzt.

### Muss die App laufen, damit ein Agent sie steuern kann?

Ja. Der Skill und die `aidaw`-CLI sprechen über MCP mit einer laufenden Instanz von AI DAW, starten Sie die App also zuerst (`aidaw status` sagt Ihnen, ob sie läuft).

## Unterstützung

### Bitte geben Sie uns einen Stern!
⭐ Ihre Unterstützung ist die treibende Kraft, die uns voranbringt!

### Trinkgeld willkommen!
<div align="center" style="display: flex; gap: 20px; justify-content: center; flex-wrap: wrap;">

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/agentparty)
[![爱发电](https://img.shields.io/badge/爱发电-支持我们-946ce6?style=for-the-badge&logo=affine&logoColor=white)](https://afdian.com/a/agentparty)

</div>

### Folgen Sie uns
<div align="center">
  <a href="https://space.bilibili.com/26978344">
    <img src="screenshots/B.png" width="100" height="100" style="border-radius: 80%; overflow: hidden;" alt="bilibili"/>
  </a>
  <a href="https://www.youtube.com/@agentParty">
    <img src="screenshots/YT.png" width="100" height="100" style="border-radius: 80%; overflow: hidden;" alt="youtube"/>
  </a>
</div>

### Treten Sie der Community bei
Wenn Sie Fragen oder Probleme mit dem Projekt haben, sind Sie herzlich willkommen, unserer Community beizutreten.

1. QQ-Gruppe: `931057213` (voll) / `902882342` (Gruppe 2)

2. Discord: [Discord-Link](https://discord.gg/f2dsAKKr2V)

## Mitwirkende

<a href="https://github.com/heshengtao/ai-daw/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=heshengtao/ai-daw" alt="Mitwirkende an heshengtao/ai-daw" />
</a>

## Lizenz

Der **Code** der Anwendung ist Apache-2.0 — siehe [LICENSE](LICENSE).

Die **Gewichte** der Modelle sind eine eigene Ebene: Sie sind niemals Teil dieses Repositories oder der Installer, sie behalten die Lizenz, die ihr Herausgeber ihnen gegeben hat, und die App lädt sie bei Bedarf herunter (und prüft die `sha256` jeder heruntergeladenen Datei). Siehe **[MODELS.md](MODELS.md)** für die vollständige Tabelle der Modelle, Quellen und Lizenzen.

Einige optionale Modelle sind **CC BY-NC 4.0 (nicht kommerziell)** — YuE2 für die Songgenerierung und
SheetSage2 für Cover-Partituren. Sie sind optionale Downloads: Verwenden Sie diese Gewichte nicht für
kommerzielle Arbeit, es sei denn, Sie besitzen eine kommerzielle Lizenz ihrer Herausgeber. Generiertes
Audio/MIDI gehört Ihnen, im Rahmen der Bedingungen der Upstream-Projekte und unter Berücksichtigung der Rechte, die Sie
an den Eingangsdaten halten.

## Lizenzen von Drittanbietern

Dieses Projekt baut auf Open-Source-Software von Drittanbietern auf. Dateien, die aus anderen Projekten übernommen wurden, und Abhängigkeiten, deren Lizenzen nicht zur permissiven Standardmenge gehören, sind in [NOTICE](NOTICE) aufgeführt.
