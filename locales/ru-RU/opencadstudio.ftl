language-system = Язык системы
language-english = English
language-turkish = Türkçe
language-dutch = Nederlands
language-french = Français
language-german = Deutsch
language-hindi = हिन्दी
language-russian = Русский
language-chinese-simplified = 简体中文

action-close = Закрыть
action-options = Параметры
action-plugins = Плагины

ribbon-tab-draw = Рисование
ribbon-tab-annotate = Аннотации
ribbon-tab-insert = Вставка
ribbon-tab-model = Модель
ribbon-tab-layout = Лист
ribbon-tab-manage = Управление
ribbon-tab-view = Вид

options-language-section = Язык
options-language-label = Язык интерфейса:
options-open-save-section = Открытие и сохранение
options-default-save-format-label = Формат сохранения по умолчанию:
options-default-save-format-help = Используется при первом сохранении нового чертежа. Существующие чертежи сохраняют тип и версию файла.
options-theme-section = Тема
options-theme-label = Тема приложения:
options-theme-help = Изменение базового цвета включает пользовательскую тему. Все оттенки компонентов создаются из этих шести цветов.
options-color-background = Фон
options-color-text = Текст
options-color-primary = Основной
options-color-success = Успех
options-color-warning = Предупреждение
options-color-danger = Опасность

command-line-ready = Open CAD Studio готов.
command-line-hint = Введите команду или используйте ленту. Откройте OBJ на вкладке «Вставка».
command-line-label = Команда:
command-line-literal-spaces = Буквальные пробелы: пробел остаётся в строке, а не запускает команду. Режим действует до отключения.

start-new-drawing = Новый чертёж
start-open-file = Открыть файл…
start-donate = Поддержать
start-send-feedback = Отправить отзыв
start-sponsors = Спонсоры
start-tutorials = Учебные материалы
start-loading-videos = Загрузка видео…
start-videos-online = Видео загружаются из Интернета.
start-open-playlist = Открыть плейлист на YouTube
start-discussions = Обсуждения
start-pinned = Закреплено
start-loading-discussions = Загрузка обсуждений…
start-discussions-online = Обсуждения загружаются с GitHub.
start-open-discussions = Открыть обсуждения на GitHub
start-supporters = Сторонники
start-support-on-patreon = Поддержать на Patreon
start-recent-files = Недавние файлы
start-videos = Видео
start-welcome = Добро пожаловать
start-recent-documents = Недавние документы
start-no-recent-files = Открытые вами файлы появятся здесь.
start-browser-storage = Хранилище браузера
start-keep-recent-files = Хранить недавние файлы

modal-about = О программе
modal-keyboard-shortcuts = Сочетания клавиш
modal-command-aliases = Псевдонимы команд
modal-find-replace = Найти и заменить
modal-plugin-manager = Диспетчер плагинов
modal-update-available = Доступно обновление
modal-layer-manager = Диспетчер слоёв
modal-layer-state-manager = Диспетчер состояний слоёв
modal-edit-layer-state = Изменить состояние слоя
modal-plot = Печать
modal-layout-manager = Диспетчер листов
modal-scale-manager = Диспетчер масштабов
modal-annotation-object-scale = Масштаб аннотативного объекта
modal-plot-style-editor = Редактор стилей печати
modal-text-style-manager = Диспетчер стилей текста
modal-multiline-style-manager = Диспетчер стилей мультилиний
modal-table-style-manager = Диспетчер стилей таблиц
modal-multileader-style-manager = Диспетчер стилей мультивыносок
modal-dimension-style-manager = Диспетчер стилей размеров
modal-default-application = Приложение по умолчанию
modal-save-warning = Предупреждение о сохранении
modal-unable-save = Не удалось сохранить чертёж
modal-drawing-changed = Чертёж изменён на диске
modal-delete-layer = Удалить слой
modal-unsaved-changes = Несохранённые изменения
modal-point-style = Стиль точек
modal-attribute-editor = Редактор атрибутов
modal-save-drawing-as = Сохранить чертёж как

command-move-base =
    MOVE  Укажите базовую точку  [{ $count ->
        [one] { $count } объект
        [few] { $count } объекта
       *[many] { $count } объектов
    }]:
command-move-target = MOVE  Укажите точку назначения  [база { $x },{ $y }]:
command-copy-array-count = COPY  Введите число элементов массива:
command-copy-base =
    COPY  Укажите базовую точку  [{ $count ->
        [one] { $count } объект
        [few] { $count } объекта
       *[many] { $count } объектов
    }]:
command-copy-array-target = COPY  Укажите вторую точку массива из { $count } элементов  [база { $x },{ $y }]:
command-copy-target =
    COPY  Укажите точку назначения  [{ $count ->
        [one] создана { $count } копия
        [few] создано { $count } копии
       *[many] создано { $count } копий
    } | Массив | Enter=готово | база { $x },{ $y }]:
