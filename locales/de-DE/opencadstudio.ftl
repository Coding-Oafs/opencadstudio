language-system = Systemsprache
language-english = English
language-turkish = Türkçe
language-dutch = Nederlands
language-french = Français
language-german = Deutsch
language-hindi = हिन्दी
language-russian = Русский
language-chinese-simplified = 简体中文

action-close = Schließen
action-options = Optionen
action-plugins = Plug-ins

ribbon-tab-draw = Zeichnen
ribbon-tab-annotate = Beschriften
ribbon-tab-insert = Einfügen
ribbon-tab-model = Modell
ribbon-tab-layout = Layout
ribbon-tab-manage = Verwalten
ribbon-tab-view = Ansicht

options-language-section = Sprache
options-language-label = Oberflächensprache:
options-open-save-section = Öffnen und Speichern
options-default-save-format-label = Standardspeicherformat:
options-default-save-format-help = Wird beim ersten Speichern einer neuen Zeichnung verwendet. Vorhandene Zeichnungen behalten Dateityp und Version bei.
options-theme-section = Design
options-theme-label = Anwendungsdesign:
options-theme-help = Das Ändern einer Grundfarbe aktiviert das benutzerdefinierte Design. Alle Komponentenfarben werden aus diesen sechs Farben erzeugt.
options-color-background = Hintergrund
options-color-text = Text
options-color-primary = Primär
options-color-success = Erfolg
options-color-warning = Warnung
options-color-danger = Gefahr

command-line-ready = Open CAD Studio ist bereit.
command-line-hint = Geben Sie einen Befehl ein oder verwenden Sie das Menüband. Öffnen Sie OBJ über die Registerkarte Einfügen.
command-line-label = Befehl:
command-line-literal-spaces = Wörtliche Leerzeichen: Das Leerzeichen bleibt in der Zeile, statt den Befehl auszuführen. Bleibt bis zum Ausschalten aktiv.

start-new-drawing = Neue Zeichnung
start-open-file = Datei öffnen…
start-donate = Spenden
start-send-feedback = Feedback senden
start-sponsors = Sponsoren
start-tutorials = Lernprogramme
start-loading-videos = Videos werden geladen…
start-videos-online = Videos werden aus dem Internet geladen.
start-open-playlist = Wiedergabeliste auf YouTube öffnen
start-discussions = Diskussionen
start-pinned = Angeheftet
start-loading-discussions = Diskussionen werden geladen…
start-discussions-online = Diskussionen werden von GitHub geladen.
start-open-discussions = Diskussionen auf GitHub öffnen
start-supporters = Unterstützer
start-support-on-patreon = Auf Patreon unterstützen
start-recent-files = Zuletzt verwendete Dateien
start-videos = Videos
start-welcome = Willkommen
start-recent-documents = Zuletzt verwendete Dokumente
start-no-recent-files = Geöffnete Dateien werden hier angezeigt.
start-browser-storage = Browserspeicher
start-keep-recent-files = Zuletzt verwendete Dateien behalten

modal-about = Info
modal-keyboard-shortcuts = Tastenkombinationen
modal-command-aliases = Befehlsaliase
modal-find-replace = Suchen und Ersetzen
modal-plugin-manager = Plug-in-Manager
modal-update-available = Aktualisierung verfügbar
modal-layer-manager = Layer-Manager
modal-layer-state-manager = Layerstatus-Manager
modal-edit-layer-state = Layerstatus bearbeiten
modal-plot = Plotten
modal-layout-manager = Layout-Manager
modal-scale-manager = Maßstabs-Manager
modal-annotation-object-scale = Maßstab des Beschriftungsobjekts
modal-plot-style-editor = Plotstileditor
modal-text-style-manager = Textstil-Manager
modal-multiline-style-manager = Mehrfachlinienstil-Manager
modal-table-style-manager = Tabellenstil-Manager
modal-multileader-style-manager = Multi-Führungslinienstil-Manager
modal-dimension-style-manager = Bemaßungsstil-Manager
modal-default-application = Standardanwendung
modal-save-warning = Speicherwarnung
modal-unable-save = Zeichnung kann nicht gespeichert werden
modal-drawing-changed = Zeichnung auf dem Datenträger geändert
modal-delete-layer = Layer löschen
modal-unsaved-changes = Nicht gespeicherte Änderungen
modal-point-style = Punktstil
modal-attribute-editor = Attribut-Editor
modal-save-drawing-as = Zeichnung speichern unter

command-move-base =
    MOVE  Basispunkt angeben  [{ $count ->
        [one] { $count } Objekt
       *[other] { $count } Objekte
    }]:
command-move-target = MOVE  Zielpunkt angeben  [Basis { $x },{ $y }]:
command-copy-array-count = COPY  Anzahl der Elemente im Array eingeben:
command-copy-base =
    COPY  Basispunkt angeben  [{ $count ->
        [one] { $count } Objekt
       *[other] { $count } Objekte
    }]:
command-copy-array-target = COPY  Zweiten Punkt für das Array mit { $count } Elementen angeben  [Basis { $x },{ $y }]:
command-copy-target =
    COPY  Zielpunkt angeben  [{ $count ->
        [one] bisher { $count } Kopie
       *[other] bisher { $count } Kopien
    } | Array | Eingabe=fertig | Basis { $x },{ $y }]:
