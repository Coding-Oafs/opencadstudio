language-system = System language
language-english = English
language-turkish = Türkçe
language-dutch = Nederlands
language-french = Français
language-german = Deutsch
language-hindi = हिन्दी
language-russian = Русский
language-chinese-simplified = Simplified Chinese

action-close = Close
action-options = Options
action-plugins = Plugins

ribbon-tab-draw = Draw
ribbon-tab-annotate = Annotate
ribbon-tab-insert = Insert
ribbon-tab-model = Model
ribbon-tab-layout = Layout
ribbon-tab-manage = Manage
ribbon-tab-view = View

options-language-section = Language
options-language-label = Interface language:
options-open-save-section = Open and Save
options-default-save-format-label = Default save format:
options-default-save-format-help = Used for the first save of a new drawing. Existing drawings keep their file type and version.
options-theme-section = Theme
options-theme-label = Application theme:
options-theme-help = Changing a base colour switches to Custom. The theme generates every component shade from these six colours.
options-color-background = Background
options-color-text = Text
options-color-primary = Primary
options-color-success = Success
options-color-warning = Warning
options-color-danger = Danger

command-line-ready = Open CAD Studio ready.
command-line-hint = Type a command or use the ribbon. Open OBJ from the Insert tab.
command-line-label = Command:
command-line-literal-spaces = Literal spaces: Space stays in the line instead of running the command. Stays on until toggled off.

start-new-drawing = New Drawing
start-open-file = Open File…
start-donate = Donate
start-send-feedback = Send Feedback
start-sponsors = Sponsors
start-tutorials = Tutorials
start-loading-videos = Loading videos…
start-videos-online = Videos load from the internet.
start-open-playlist = Open playlist on YouTube
start-discussions = Discussions
start-pinned = Pinned
start-loading-discussions = Loading discussions…
start-discussions-online = Discussions load from GitHub.
start-open-discussions = Open Discussions on GitHub
start-supporters = Supporters
start-support-on-patreon = Support on Patreon
start-recent-files = Recent Files
start-videos = Videos
start-welcome = Welcome
start-recent-documents = Recent Documents
start-no-recent-files = Files you open will show up here.
start-browser-storage = Browser storage
start-keep-recent-files = Keep recent files

modal-about = About
modal-keyboard-shortcuts = Keyboard Shortcuts
modal-command-aliases = Command Aliases
modal-find-replace = Find and Replace
modal-plugin-manager = Plugin Manager
modal-update-available = Update Available
modal-layer-manager = Layer Manager
modal-layer-state-manager = Layer State Manager
modal-edit-layer-state = Edit Layer State
modal-plot = Plot
modal-layout-manager = Layout Manager
modal-scale-manager = Scale Manager
modal-annotation-object-scale = Annotation Object Scale
modal-plot-style-editor = Plot Style Editor
modal-text-style-manager = Text Style Manager
modal-multiline-style-manager = Multiline Style Manager
modal-table-style-manager = Table Style Manager
modal-multileader-style-manager = Multileader Style Manager
modal-dimension-style-manager = Dimension Style Manager
modal-default-application = Default Application
modal-save-warning = Save Warning
modal-unable-save = Unable to Save Drawing
modal-drawing-changed = Drawing Changed on Disk
modal-delete-layer = Delete Layer
modal-unsaved-changes = Unsaved Changes
modal-point-style = Point Style
modal-attribute-editor = Attribute Editor
modal-save-drawing-as = Save Drawing As

command-move-base =
    MOVE  Specify base point  [{ $count ->
        [one] { $count } object
       *[other] { $count } objects
    }]:
command-move-target = MOVE  Specify destination  [base { $x },{ $y }]:
command-copy-array-count = COPY  Enter number of items to array:
command-copy-base =
    COPY  Specify base point  [{ $count ->
        [one] { $count } object
       *[other] { $count } objects
    }]:
command-copy-array-target = COPY  Specify second point for { $count }-item array  [base { $x },{ $y }]:
command-copy-target =
    COPY  Specify destination  [{ $count ->
        [one] { $count } copy so far
       *[other] { $count } copies so far
    } | Array | Enter=done | base { $x },{ $y }]:
