language-system = Langue du système
language-english = English
language-turkish = Türkçe
language-dutch = Nederlands
language-french = Français
language-german = Deutsch
language-hindi = हिन्दी
language-russian = Русский
language-chinese-simplified = 简体中文

action-close = Fermer
action-options = Options
action-plugins = Extensions

ribbon-tab-draw = Dessin
ribbon-tab-annotate = Annoter
ribbon-tab-insert = Insérer
ribbon-tab-model = Modèle
ribbon-tab-layout = Présentation
ribbon-tab-manage = Gérer
ribbon-tab-view = Affichage

options-language-section = Langue
options-language-label = Langue de l'interface :
options-open-save-section = Ouvrir et enregistrer
options-default-save-format-label = Format d'enregistrement par défaut :
options-default-save-format-help = Utilisé lors du premier enregistrement d'un nouveau dessin. Les dessins existants conservent leur type de fichier et leur version.
options-theme-section = Thème
options-theme-label = Thème de l'application :
options-theme-help = La modification d'une couleur de base active le thème personnalisé. Le thème génère toutes les nuances des composants à partir de ces six couleurs.
options-color-background = Arrière-plan
options-color-text = Texte
options-color-primary = Principale
options-color-success = Succès
options-color-warning = Avertissement
options-color-danger = Danger

command-line-ready = Open CAD Studio est prêt.
command-line-hint = Saisissez une commande ou utilisez le ruban. Ouvrez un fichier OBJ depuis l'onglet Insérer.
command-line-label = Commande :
command-line-literal-spaces = Espaces littéraux : l'espace reste dans la ligne au lieu d'exécuter la commande. Ce mode reste actif jusqu'à sa désactivation.

start-new-drawing = Nouveau dessin
start-open-file = Ouvrir un fichier…
start-donate = Faire un don
start-send-feedback = Envoyer des commentaires
start-sponsors = Sponsors
start-tutorials = Tutoriels
start-loading-videos = Chargement des vidéos…
start-videos-online = Les vidéos sont chargées depuis Internet.
start-open-playlist = Ouvrir la playlist sur YouTube
start-discussions = Discussions
start-pinned = Épinglé
start-loading-discussions = Chargement des discussions…
start-discussions-online = Les discussions sont chargées depuis GitHub.
start-open-discussions = Ouvrir les discussions sur GitHub
start-supporters = Contributeurs
start-support-on-patreon = Soutenir sur Patreon
start-recent-files = Fichiers récents
start-videos = Vidéos
start-welcome = Bienvenue
start-recent-documents = Documents récents
start-no-recent-files = Les fichiers que vous ouvrez apparaîtront ici.
start-browser-storage = Stockage du navigateur
start-keep-recent-files = Conserver les fichiers récents

modal-about = À propos
modal-keyboard-shortcuts = Raccourcis clavier
modal-command-aliases = Alias de commandes
modal-find-replace = Rechercher et remplacer
modal-plugin-manager = Gestionnaire d'extensions
modal-update-available = Mise à jour disponible
modal-layer-manager = Gestionnaire de calques
modal-layer-state-manager = Gestionnaire d'états de calque
modal-edit-layer-state = Modifier l'état du calque
modal-plot = Tracer
modal-layout-manager = Gestionnaire de présentations
modal-scale-manager = Gestionnaire d'échelles
modal-annotation-object-scale = Échelle de l'objet annotatif
modal-plot-style-editor = Éditeur de styles de tracé
modal-text-style-manager = Gestionnaire de styles de texte
modal-multiline-style-manager = Gestionnaire de styles de multilignes
modal-table-style-manager = Gestionnaire de styles de tableau
modal-multileader-style-manager = Gestionnaire de styles de lignes de repère multiples
modal-dimension-style-manager = Gestionnaire de styles de cote
modal-default-application = Application par défaut
modal-save-warning = Avertissement d'enregistrement
modal-unable-save = Impossible d'enregistrer le dessin
modal-drawing-changed = Dessin modifié sur le disque
modal-delete-layer = Supprimer le calque
modal-unsaved-changes = Modifications non enregistrées
modal-point-style = Style de point
modal-attribute-editor = Éditeur d'attributs
modal-save-drawing-as = Enregistrer le dessin sous

command-move-base =
    MOVE  Spécifiez le point de base  [{ $count ->
        [one] { $count } objet
       *[other] { $count } objets
    }] :
command-move-target = MOVE  Spécifiez la destination  [base { $x },{ $y }] :
command-copy-array-count = COPY  Saisissez le nombre d'éléments du réseau :
command-copy-base =
    COPY  Spécifiez le point de base  [{ $count ->
        [one] { $count } objet
       *[other] { $count } objets
    }] :
command-copy-array-target = COPY  Spécifiez le second point du réseau de { $count } éléments  [base { $x },{ $y }] :
command-copy-target =
    COPY  Spécifiez la destination  [{ $count ->
        [one] { $count } copie jusqu'ici
       *[other] { $count } copies jusqu'ici
    } | Réseau | Entrée=terminer | base { $x },{ $y }] :
