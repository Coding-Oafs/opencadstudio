language-system = Systeemtaal
language-english = English
language-turkish = Türkçe
language-dutch = Nederlands
language-french = Français
language-german = Deutsch
language-hindi = हिन्दी
language-russian = Русский
language-chinese-simplified = 简体中文

action-close = Sluiten
action-options = Opties
action-plugins = Plug-ins

ribbon-tab-draw = Tekenen
ribbon-tab-annotate = Annoteren
ribbon-tab-insert = Invoegen
ribbon-tab-model = Model
ribbon-tab-layout = Lay-out
ribbon-tab-manage = Beheren
ribbon-tab-view = Beeld

options-language-section = Taal
options-language-label = Interfacetaal:
options-open-save-section = Openen en opslaan
options-default-save-format-label = Standaard opslagformaat:
options-default-save-format-help = Wordt gebruikt wanneer een nieuwe tekening voor het eerst wordt opgeslagen. Bestaande tekeningen behouden hun bestandstype en versie.
options-theme-section = Thema
options-theme-label = Toepassingsthema:
options-theme-help = Als u een basiskleur wijzigt, wordt het aangepaste thema geactiveerd. Het thema maakt alle componenttinten op basis van deze zes kleuren.
options-color-background = Achtergrond
options-color-text = Tekst
options-color-primary = Primair
options-color-success = Geslaagd
options-color-warning = Waarschuwing
options-color-danger = Gevaar

command-line-ready = Open CAD Studio is gereed.
command-line-hint = Typ een opdracht of gebruik het lint. Open OBJ via het tabblad Invoegen.
command-line-label = Opdracht:
command-line-literal-spaces = Letterlijke spaties: een spatie blijft in de regel staan in plaats van de opdracht uit te voeren. Blijft actief tot u dit uitschakelt.

start-new-drawing = Nieuwe tekening
start-open-file = Bestand openen…
start-donate = Doneren
start-send-feedback = Feedback verzenden
start-sponsors = Sponsors
start-tutorials = Zelfstudies
start-loading-videos = Video's laden…
start-videos-online = Video's worden via internet geladen.
start-open-playlist = Afspeellijst openen op YouTube
start-discussions = Discussies
start-pinned = Vastgezet
start-loading-discussions = Discussies laden…
start-discussions-online = Discussies worden geladen vanaf GitHub.
start-open-discussions = Discussies openen op GitHub
start-supporters = Ondersteuners
start-support-on-patreon = Ondersteunen op Patreon
start-recent-files = Recente bestanden
start-videos = Video's
start-welcome = Welkom
start-recent-documents = Recente documenten
start-no-recent-files = Bestanden die u opent, verschijnen hier.
start-browser-storage = Browseropslag
start-keep-recent-files = Recente bestanden bewaren

modal-about = Info
modal-keyboard-shortcuts = Sneltoetsen
modal-command-aliases = Opdrachtaliassen
modal-find-replace = Zoeken en vervangen
modal-plugin-manager = Plug-inbeheer
modal-update-available = Update beschikbaar
modal-layer-manager = Lagenbeheer
modal-layer-state-manager = Laagstatusbeheer
modal-edit-layer-state = Laagstatus bewerken
modal-plot = Afdrukken
modal-layout-manager = Lay-outbeheer
modal-scale-manager = Schaalbeheer
modal-annotation-object-scale = Schaal van annotatieobject
modal-plot-style-editor = Afdrukstijleditor
modal-text-style-manager = Tekststijlbeheer
modal-multiline-style-manager = Meervoudige-lijnstijlbeheer
modal-table-style-manager = Tabelstijlbeheer
modal-multileader-style-manager = Multileaderstijlbeheer
modal-dimension-style-manager = Maatstijlbeheer
modal-default-application = Standaardtoepassing
modal-save-warning = Opslagwaarschuwing
modal-unable-save = Tekening kan niet worden opgeslagen
modal-drawing-changed = Tekening gewijzigd op schijf
modal-delete-layer = Laag verwijderen
modal-unsaved-changes = Niet-opgeslagen wijzigingen
modal-point-style = Puntstijl
modal-attribute-editor = Kenmerkeditor
modal-save-drawing-as = Tekening opslaan als

command-move-base =
    MOVE  Geef het basispunt op  [{ $count ->
        [one] { $count } object
       *[other] { $count } objecten
    }]:
command-move-target = MOVE  Geef de bestemming op  [basis { $x },{ $y }]:
command-copy-array-count = COPY  Voer het aantal items in de matrix in:
command-copy-base =
    COPY  Geef het basispunt op  [{ $count ->
        [one] { $count } object
       *[other] { $count } objecten
    }]:
command-copy-array-target = COPY  Geef het tweede punt voor de matrix met { $count } items op  [basis { $x },{ $y }]:
command-copy-target =
    COPY  Geef de bestemming op  [{ $count ->
        [one] { $count } kopie tot nu toe
       *[other] { $count } kopieën tot nu toe
    } | Matrix | Enter=gereed | basis { $x },{ $y }]:
