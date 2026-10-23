# kadr — техническая спецификация целевой версии

Статус: действующая. Язык документа — русский, идентификаторы и код — английские.
Назначение: описать архитектуру, в которой UI-фреймворк — заменяемый слой, покрыть
проект автотестами и зафиксировать принятые решения.

**Решение по фреймворку (2026-10-23): остаёмся на GPUI CE.** Спайк (раздел 9) показал, что iced
не даёт выигрыша на Linux, Tauri проигрывает вдвое по памяти и старту, а GPUI CE честно прошёл
desktop-e2e на macOS и Windows (на Windows — на WARP без видеокарты). Разделение на крейты
сохраняется как страховка: если GPUI CE перестанет развиваться, переписывается только `kadr-ui-gpui`.

## 0. Идентификаторы проекта

| Что | Значение |
|---|---|
| Имя продукта | kadr |
| Лицензия | MIT, `Copyright (c) 2026 Evgeniy Vlasov` |
| Репозиторий | `github.com/vlasmn/kadr` (публичный); при переносе — организация `kadr-sh` |
| Домен | `kadr.sh`; сайт — GitHub Pages; релизы и апдейтер — `https://kadr.sh/releases` → GitHub Releases |
| Bundle id | `sh.kadr.app` (macOS), `sh.kadr.Kadr` (desktop entry / Flatpak), тот же корень в winget |
| AUR | `kadr-sh` (из тарбола), `kadr-sh-bin`, `kadr-sh-git`; `provides=('kadr') conflicts=('kadr')` — имя `kadr` занято другим проектом, бинарник `/usr/bin/kadr` конфликтует |
| crates.io | не публикуем (имя `kadr` занято сквотом; GUI распространяется бинарниками) |

---

## 1. Цели и принципы

1. **Кроссплатформенность без компромиссов.** Linux (Wayland/Hyprland — основной, X11 — best effort),
   macOS, Windows. Одна кодовая база, одинаковое поведение, платформенные различия — только в `kadr-platform`.
2. **Ресурсы.** В простое живёт один маленький процесс без UI-фреймворка. Всё тяжёлое поднимается по
   требованию и гасится. Бюджеты — в разделе 2, проверяются автоматически.
3. **UI — заменяемый слой.** Ни один крейт, кроме `kadr-ui-*`, не импортирует типы UI-фреймворка.
   Всё, что можно проверить без окна, проверяется без окна.
4. **Детерминизм.** Растер аннотаций, экспорт видео, bundle — воспроизводимы байт-в-байт
   (или в пределах порога для растера), иначе golden-тесты невозможны.
5. **Код пишет ИИ, принимает человек.** Поэтому: одна команда для всех проверок, машиночитаемый
   результат (JSON, diff-картинки), тестовые хуки в самом приложении, задокументированные ловушки.

## 2. Бюджеты (проверяются `just bench`, падают при превышении)

| Метрика | Цель | Как мерить |
|---|---|---|
| RSS `kadr tray` через 5 мин простоя | ≤ 15 МБ | `/proc/<pid>/status` VmRSS; mac `ps -o rss`; win `Get-Process` |
| RSS главного процесса, редактор с 4K PNG | ≤ 250 МБ | то же |
| Idle CPU главного процесса с открытым окном, без ввода, 30 с | ≤ 0.5 % | `pidstat`/`top -b` |
| Хоткей → оверлей виден, процесс жив (тёплый) | ≤ 80 мс | лог-метка в tray при хоткее → `dev shot` фиксирует первый кадр с оверлеем |
| Хоткей → оверлей виден, холодный старт | ≤ 300 мс | то же |
| `kadr edit <png>` → окно с картинкой | ≤ 400 мс | лог-метки |
| Размер бинаря профиля `dist` (strip) | ≤ 25 МБ — предупреждение, ≤ 30 МБ — ошибка | `stat` |
| `cargo build --release` инкрементально (1 файл UI) | ≤ 20 с | `time` |

Профили: `--release` — без fat LTO, инкрементальный, для разработки и бюджета сборки;
`dist` — `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, для релизов и бюджета размера.
Бюджет размера не ужимать ценой функциональности: 25 МБ — ориентир, не цель.

База и история измерений — `docs/perf.md`. Бенч гоняется только во вложенном Hyprland (6.4),
в живой сессии цифры недостоверны.

## 3. Архитектура: Cargo workspace

```
kadr/
  Cargo.toml              # workspace
  crates/
    kadr-core/            # модель данных и логика, 0 зависимостей от OS и UI
    kadr-raster/          # растеризация аннотаций: tiny-skia, resvg, ab_glyph
    kadr-platform/        # trait Platform + linux/macos/windows
    kadr-media/           # ffmpeg/libplacebo/gpu-screen-recorder, модель студии
    kadr-ocr/             # tesseract
    kadr-ipc/             # протокол single-instance и dev-хуков
    kadr-tray/            # демон: трей + глобальные хоткеи, без UI-фреймворка
    kadr-overlay/         # оверлей выделения: winit + softbuffer (см. 3.8)
    kadr-ui/              # trait Shell + общие view-model'и, без фреймворка
    kadr-ui-gpui/         # реализация Shell на GPUI CE — единственная оболочка
    kadr-mcp/             # MCP-сервер поверх kadr-ipc (раздел 11)
  src/main.rs             # бинарь `kadr`: CLI
  tests/                  # интеграционные и golden
  scripts/                # bench, e2e, headless-compositor
  docs/                   # SPEC.ru.md (этот файл), perf.md, traps.md
```

Правило зависимостей (проверяется `cargo deny` / `scripts/check-deps.sh`):
`core ← raster, media, ocr, ipc ← platform ← ui ← ui-*`. Стрелка назад запрещена.
`gpui`, `tauri`, `iced` и т. п. встречаются только в `kadr-ui-*` и `Cargo.toml` бинаря.

### 3.1 `kadr-core`

- `geometry`: `Rect`, `Point`, `Size` в логических и физических пикселях с явным `Scale`;
  снап к границам окон/мониторов; нормализация выделения.
- `document`: модель аннотаций редактора. Версионированный JSON (`"version": N`), миграции между
  версиями. Типы: `Arrow`, `Rect`, `Ellipse`, `Line`, `Pen`, `Text`, `Counter`, `Blur`, `Pixelate`,
  `Highlight`, `Crop`, `Redact`. Каждый — чистые данные (координаты, стиль, z-order), без методов отрисовки.
- `history`: undo/redo как список команд над `Document`; инварианты проверяются тестами.
- `settings`: типизированные настройки + сериализация, значения по умолчанию, миграции.
- `presets`: сохранённые «looks» (общие для редактора и студии).
- `bundle`: формат `.kadr` (tar, `manifest.json` первым). Запись/чтение, round-trip-тест.
- `i18n`: ключи, `t()`/`tr!()`, тест на совпадение ключей и слотов `en.json`/`ru.json`.
- `storage`: пути (cache, config, data), именование файлов, история снимков (индекс).

### 3.2 `kadr-raster`

Единственный способ получить картинку из `Document`:
`render(doc: &Document, base: &RgbaImage, opts: RenderOpts) -> RgbaImage`.

- Детерминирован: фиксированные шрифты (bundled IBM Plex), без системных шрифтов, без времени.
- Тот же код используется редактором для превью на канвасе и для экспорта — расхождений нет по построению.
- Метрики текста: только через `raster::text::em()`, чтобы измеренное и нарисованное совпадали.
- Поддержка масштаба (1×, 2×) как параметр, не глобал.

### 3.3 `kadr-platform`

```rust
pub trait Platform: Send + Sync {
    fn monitors(&self) -> Vec<Monitor>;             // id, bounds, scale, name, primary
    fn windows(&self) -> Vec<WindowInfo>;           // для picker'а окон
    fn capture(&self, target: CaptureTarget) -> Result<Capture>;  // Screen|Window|Region
    fn clipboard(&self) -> &dyn Clipboard;          // image + text + file
    fn notify(&self, n: Notification) -> Result<()>;
    fn trash(&self, path: &Path) -> Result<()>;
    fn open_with_default(&self, path: &Path) -> Result<()>;
    fn prefers_dark(&self) -> bool;
    fn cursor_shapes(&self) -> Option<Box<dyn CursorShapes>>;  // для студии
    fn autostart(&self, enabled: bool) -> Result<()>;
    fn permissions(&self) -> Permissions;           // macOS: screen recording, accessibility
    fn window_controls(&self) -> WindowControls;    // traffic lights / caption buttons / none
}
```

- `linux`: xdg-desktop-portal (`ashpd`) как основной путь захвата; Hyprland-специфика (hyprctl,
  layer-shell, Lua-хук кликов/клавиш) — за фичей `hyprland`; X11 — через `xcap`.
- `macos`: ScreenCaptureKit через `objc2-*`; проверка и запрос прав; `NSStatusItem` в tray-крейте.
- `windows`: Windows.Graphics.Capture через `windows` crate (fallback `xcap`).
- **Курсоры в студии** (`CursorShapes`): на всех ОС во время записи берутся системные — Linux из темы
  пользователя (XCursor), macOS через `NSCursor`, Windows через `GetCursorInfo`/`DrawIconEx`. В репозитории
  лежит только собственный запасной набор `data/cursors/*.svg` (оригинальные рисунки kadr) — для систем без
  темы и для golden-тестов. Чужие наборы курсоров не вкладываем: лицензии (GPL) и чужой дизайн.
- Всё, что не зависит от OS, но зависит от «десктопа» (arboard, notify-rust, trash) — в `desktop.rs`.
- Для тестов: `FakePlatform` с заданными мониторами/окнами и захватом из PNG-фикстур.

### 3.4 `kadr-media`

- Модель студии: таймлайн, зумы, cuts, trim, курсор (траектория + формы), экспортные пресеты — чистые данные.
- `layout`: расчёт кадра (камера, кроп) — чистые функции, покрыты тестами (уже есть).
- Исполнители: обёртки над `ffmpeg`/`libplacebo`/`gpu-screen-recorder` как `Job` с прогрессом по каналу.
  Команды формируются чистыми функциями (`fn ffmpeg_args(&Export) -> Vec<String>`) и тестируются
  как строки; сами процессы — за трейтом `Runner`, в тестах — `FakeRunner`.
- Инварианты libplacebo (целые чётные кропы) — в типах (`EvenSize`), не в комментариях.

### 3.5 `kadr-ipc`

- Один сокет (Unix socket / named pipe), JSON-lines. Первый процесс — сервер, остальные вызовы `kadr …`
  пересылают аргументы и завершаются.
- Публичные команды = CLI: `capture {area|window|screen|all}`, `record …`, `edit <png>`, `studio <mp4>`,
  `open <png>`, `settings`, `history`, `pin <png>`, `ocr <png>`, `tray`, `install-desktop`.
- **Dev-хуки** (только с `--features dev` или `KADR_DEV=1`), раздел 6.3.

### 3.6 `kadr-tray`

- Отдельный бинарь/процесс без UI-фреймворка: иконка (`tray-icon` / `ksni`), меню, глобальные хоткеи
  (`global-hotkey`; на Hyprland — через биндинги компositora), автостарт.
- По хоткею: если главный процесс жив — IPC-команда; иначе — spawn `kadr <cmd>`.
  Пишет в лог метку `hotkey_at=<monotonic ms>` для бенча латентности.
- Держит ли главный процесс «тёплым» — настройка (`keep_warm: bool`, по умолчанию off на Linux,
  on на macOS/Windows, если холодный старт не укладывается в бюджет).

### 3.7 `kadr-ui` (без фреймворка)

```rust
pub trait Shell {
    fn open_editor(&self, session: EditorSession) -> WindowHandle;
    fn open_studio(&self, session: StudioSession) -> WindowHandle;
    fn open_settings(&self) -> WindowHandle;
    fn show_card(&self, card: Card) -> WindowHandle;        // quick-access карточка
    fn show_panel(&self, panel: RecordPanel) -> WindowHandle;
    fn pin(&self, image: PinImage) -> WindowHandle;
    fn run(self, ipc: IpcServer) -> !;                      // event loop, quit on idle
}
```

- View-model'и (`EditorVm`, `StudioVm`, `SettingsVm`, `CardVm`) живут здесь: состояние, команды,
  undo, выбранный инструмент, hit-testing по `Document`. UI-фреймворк только рисует и шлёт события.
  Это позволяет тестировать поведение редактора без окна (раздел 6.1) и делает замену фреймворка
  переписыванием «вью», а не логики.
- Тема и палитра (`Sumi Dark` / light), размеры (`REM`, `SCALE`), иконки (Lucide, bundled) — здесь,
  как данные. Отрисовка компонентов — в `kadr-ui-*`.

### 3.8 `kadr-overlay`

Оверлей выделения области — отдельный крейт на `winit` + `softbuffer` (CPU-рендер, без GPU-контекста):
затемнение, рамка, размеры, лупа, снап к окнам, hint-клавиши. Причины:

- латентность: нет инициализации GPU/webview, окно и первый кадр за десятки миллисекунд;
- независимость от UI-фреймворка — оверлей один для всех кандидатов спайка;
- на Linux без Vulkan оверлей всё равно работает.

На Wayland — `layer-shell` на весь монитор + input region для click-through; на macOS — `NSPanel`
уровня `screenSaver` без активации; на Windows — layered topmost window. Если в ходе спайка
окажется, что фреймворк-кандидат делает оверлей не хуже по бюджету — крейт остаётся как fallback.

### 3.9 `kadr-ui-gpui`

Реализация `Shell`. Только отрисовка, события, окна. Никакой бизнес-логики.
Правило «ни одного gpui-типа вне этого крейта» остаётся в силе и проверяется `scripts/check-deps.sh`:
это страховка на случай, если GPUI CE перестанет развиваться.

### 3.10 Платформенные края: нативность без нативных виджетов

UI рисуется кастомно и одинаково на всех ОС (как у Zed и Raycast). Ощущение нативности
достигается на краях, где приложение встречается с системой. Это обязательные требования, а не пожелания:

- **Системные диалоги** открытия/сохранения/выбора папки — только `rfd`, никаких своих.
- **Нативные меню**: на macOS (menu bar и контекстные) и Windows — через `muda`; на Linux/Wayland
  системных меню вне GTK/Qt нет, там меню рисует kadr.
- **Клавиатурные конвенции** — таблица в `kadr-platform::keymap`: Cmd/Ctrl, `Cmd+,` настройки,
  `Cmd+W`/`Cmd+Q` на macOS, `Alt+F4`/`Ctrl+,` на Windows; `Home/End` vs `Cmd+←/→`,
  `Option+←/→` по словам. Подсказки клавиш (`KeyBinding`) рисуют символы ОС (⌘⌥⇧ / Ctrl Alt Shift).
- **Текстовый ввод**: IME, эмодзи-пикер системы, выделение по словам — проверяются в desktop-e2e
  на macOS и Windows.
- **Платформенные константы стиля** в `kadr-platform::style`: радиус окна (macOS 10–12, Windows 11 8,
  Linux по компositору), тип переключателя (Switch на macOS, по-умолчанию Switch везде), порядок кнопок
  в диалогах (подтверждение справа на macOS, слева на Windows), оконные контролы (уже есть).
- **Поведение окон**: редактор и настройки ужимаются под экран и центрируются на любом разрешении
  (ловушка из спайка: окно 1360×760 на экране 1024×768 уехало за край); тест в desktop-e2e.
- **Системный шрифт**, тёмная тема за системой, нативные трей и уведомления — уже есть, не ломать.
- **Доступность** (screen reader) — известный долг кастомного UI на GPUI CE; фиксируется в `docs/traps.md`,
  не блокирует релиз.

## 4. Процессная модель

```
[kadr tray]  — всегда; ≤15 МБ; хоткеи, иконка, автостарт
     │ IPC / spawn
[kadr]       — по требованию; Shell + overlay; quit через N с без окон и jobs
     │ spawn
[ffmpeg | gpu-screen-recorder | tesseract] — отдельные процессы, Job с прогрессом
```

- Логи: `tracing` в файл `cache_dir()/kadr.log` + `--log-json` для бенчей.
- Крэш главного процесса не трогает трей и не теряет запись (recorder пишет напрямую в файл,
  манифест студии сохраняется инкрементально).

## 5. Контракты данных

- `Document` JSON: schema в `docs/schema/document.v<N>.json`, генерируется из типов (`schemars`),
  тест проверяет, что фикстуры всех версий читаются.
- `.kadr` bundle: `manifest.json` (`version`, `kind: screenshot|recording`, пути), затем файлы.
- `settings.json`: версия + миграции; тест «настройки v1 читаются v<N>».
- IPC: `{"cmd": "...", "args": {...}, "id": n}` → `{"id": n, "ok": true, "result": ...}`.

## 6. Автотесты

Запуск всего: `just test` (= `just unit && just golden && just ui && just e2e`). Каждый слой — отдельная
команда, чтобы ИИ гонял нужный слой быстро. Результат каждого падения — путь к артефакту
(diff-png, json-дамп, лог), который агент открывает сам.

### 6.1 Unit (`just unit` = `cargo test --workspace --exclude kadr-ui-*`)

- `kadr-core`: геометрия/снап (property-тесты `proptest`: нормализация идемпотентна, снап не выходит
  за монитор), undo/redo (после `n` do + `n` undo документ равен исходному), bundle round-trip,
  миграции схем, i18n-ключи, пресеты.
- `kadr-ui` view-model'и: сценарии «выбрал стрелку → drag → документ содержит Arrow с такими координатами»,
  «hit-test попадает в верхний по z», «Esc сбрасывает инструмент», «Ctrl+Z после текста убирает текст».
  Без окна, без фреймворка — это основной способ тестировать поведение редактора.
- `kadr-media`: `ffmpeg_args` по snapshot (`insta`), layout камеры, инварианты чётных кропов.
- `kadr-platform`: парсеры (`hyprctl -j`, `xdg` ответы) на фикстурах; `FakePlatform`.
- Целевое время: `just unit` ≤ 60 с.

### 6.2 Golden растера (`just golden` = `cargo test -p kadr-raster --test golden`)

- `tests/golden/cases/<name>/{base.png, doc.json, expected.png}`; один кейс на инструмент + комбинации
  (текст на блюре, стрелка через кроп, counter 1–99, 2× scale).
- Сравнение перцептивное с порогом (`image-compare` SSIM ≥ 0.995 или `dssim` ≤ 0.001), плюс лимит
  на число пикселей с |Δ| > 8. Побайтно — только для `Blur`/`Pixelate` (там детерминизм обязателен).
- При падении пишет `target/golden/<name>/{actual.png, diff.png}`. Обновление эталонов —
  только `UPDATE_GOLDEN=1 just golden`, и в коммите обязательно упоминание «golden updated: <cases>»
  — это сигнал человеку посмотреть diff.
- Шрифты — только bundled; тест падает, если растер обратился к системному шрифту.

### 6.3 Dev-хуки и UI-тесты без компositora (`just ui`)

Хуки в приложении (фича `dev`), доступны через IPC и CLI `kadr dev …`:

| Команда | Результат |
|---|---|
| `dev windows` | список окон: id, kind, bounds, scale |
| `dev dump [window]` | дерево элементов: `test_id`, bounds, text, state (hovered/pressed/disabled/checked), scroll |
| `dev find <test_id>` | один элемент |
| `dev click <test_id> [--button] [--at dx,dy]` | синтетический клик в центр элемента |
| `dev hover <test_id>` / `dev unhover` | синтетический hover (без курсора) |
| `dev key <combo>` / `dev type <text>` | ввод |
| `dev drag <id> <dx,dy>` | drag от элемента |
| `dev shot <path> [window]` | скриншот окна средствами фреймворка (или компositora) |
| `dev state` | JSON view-model'и текущего окна (документ, инструмент, undo-глубина) |
| `dev set-theme dark\|light`, `dev set-scale 1\|2` | для детерминизма |

- Каждый интерактивный элемент получает `test_id` (`editor.tool.arrow`, `settings.hotkeys.area`).
  Линтер-тест: все `Button`/`IconButton`/`Switch` в `kadr-ui-*` создаются с `test_id`.
- Тесты в `tests/ui/*.rs` запускают `kadr --features dev` с `FakePlatform`, работают через хуки и
  проверяют `dev state`/`dev dump`. Скриншоты здесь не сравниваются — только структура и состояние.
- Если в GPUI CE сохранён `test-support` (`TestAppContext`), часть этих тестов может идти in-process
  без окна; это оптимизация, контракт — хуки.

### 6.4 E2E на компositore (`just e2e`, Linux; macOS/Windows — desktop-e2e в CI, раздел 7)

- `scripts/nested.sh` поднимает вложенный Hyprland с тестовым конфигом (`tests/e2e/hypr.conf`):
  **headless-выход**, не окно в special-workspace (иначе кадры зависят от того, что открыто у пользователя),
  1920×1080 @1×, без анимаций, без блюра, фиксированные шрифты и курсор-тема,
  `kadr` из `target/release`. Переменные `WAYLAND_DISPLAY`/`HYPRLAND_INSTANCE_SIGNATURE` отдаются тестам.
- Сценарии (`tests/e2e/*.sh` или Rust-тесты с `std::process`):
  1. tray запущен → хоткей (через `hyprctl dispatch`) → оверлей виден ≤ бюджета → drag → PNG в истории,
     размер и содержимое как у `grim -g` той же области (SSIM ≥ 0.99).
  2. `kadr edit fixture.png` → `dev click editor.tool.arrow` → `dev drag` → `dev key ctrl+s` →
     файл равен golden-растеру того же документа (связь слоёв 6.2 и 6.4).
  3. `kadr record area …` 3 с → `kadr studio` → экспорт → `ffprobe` даёт ожидаемые размеры/длительность.
  4. settings: переключить тему → `dev shot` → сравнение с эталоном с маской на системном шрифте.
  5. single-instance: второй `kadr edit` переиспользует процесс (один PID).
- Скриншоты экранов сравниваются перцептивно, как в 6.2, эталоны хранятся на 1× и 2×.

### 6.5 Бенчи (`just bench`)

- `scripts/bench.sh` снимает метрики раздела 2 в `target/bench.json`, сравнивает с `docs/perf.md`
  (база) и бюджетами; падает при превышении бюджета, предупреждает при регрессии > 15 % от базы.
- Запускается в e2e-окружении (тот же headless Hyprland), по 5 прогонов, медиана.

### 6.6 Статические проверки (`just lint`)

`cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny` (лицензии: GPL запрещён, Apache/MIT/ISC/OFL ок),
`scripts/check-deps.sh` (граф зависимостей из раздела 3), тест i18n-ключей, тест `test_id`.

## 7. CI

- Репозиторий публичный — минуты Actions бесплатны. GitHub Actions, матрица `ubuntu` (unit, golden,
  lint, e2e, bench), `macos`, `windows` (unit, golden, lint, build `dist`).
- `desktop-e2e` (macOS + Windows): открывает редактор на рабочем столе раннера, настоящим указателем
  выбирает инструмент, рисует, сохраняет; проверяет, что окно целиком на экране и что инструмент
  реально выбран (цвет кнопки через `kadr-shot pixel`). `result.json`: `outcome` ∈ `passed | failed |
  gpu_unavailable`, `renderer`, `gpu` (адаптер; WARP узнаётся по «Basic Render Driver»), координаты окна и
  кликов. `gpu_unavailable` — не провал фреймворка, а повод для прогона на живой машине. Запуск по
  `workflow_dispatch` и по тегу.
- Артефакты при падении: `target/golden/**`, `target/e2e/**` (shots, dumps, logs), `target/bench.json`,
  `desktop-e2e-<os>` (скриншоты шагов, `app.log`, `result.json`).
- PR без изменений в `kadr-ui-gpui` не гоняет e2e (path filter) — для скорости.
- Релиз по тегу `v*`: сборка `dist` на трёх ОС, артефакты в GitHub Releases, обновление AUR-пакетов
  `kadr-sh-bin` и Homebrew tap.

## 8. Соглашения (переносятся из CLAUDE.md)

- i18n по ключам, обе локали, тест на ключи. Lucide-иконки, OFL-шрифты, ничего из Zed, никакого GPL.
- Концентрические скругления, пространство вместо линий, акцент на главном действии. Без «стекла».
- Новая OS-специфика — только в `kadr-platform`. Новая логика экрана — в `kadr-ui` (view-model), не во «вью».
- `docs/traps.md` — живой список ловушек фреймворка; при миграции заводится новый раздел.
- Коммиты в `main`, трейлер `Co-Authored-By` из сессии.

## 9. Эксперимент по миграции — завершён

**Итог (2026-10-23): остаёмся на GPUI CE.** Ветки `spike/*` удалены после записи результатов
в `docs/perf.md`. Ниже — протокол, по которому решение принималось; он остаётся на случай повторного
вопроса (например, если GPUI CE перестанет развиваться).

Сводка спайка (Linux, вложенный Hyprland, редактор с 4K-картинкой):

| | GPUI CE | iced 0.14 | Tauri 2.12 |
|---|---|---|---|
| Открытие редактора | 304 мс | 179 мс | 671 мс |
| Память редактора | 263 МБ | 301 МБ (+14 %) | 564 МБ (+114 %) |
| CPU в простое | 0.1 % | 0.1 % | 0 % |
| Бинарник `dist` | 25.0 МБ | 17.6 МБ | 7.6 МБ + системный WebKit |
| desktop-e2e macOS / Windows | passed / passed (WARP) | passed / passed | — |

Tauri отпал по метрикам Linux. iced — ровня, выигрыша нет, а переписывание редактора и студии
не окупается. GPUI CE на macOS и Windows работает, включая раннер без видеокарты.

### 9.1 Фаза 0 — база (до любых изменений UI)

1. Снять метрики раздела 2 на текущем билде → `docs/perf.md`.
2. Golden-тесты растера на текущем `editor.rs` (до выделения крейтов — это защита рефакторинга).
3. Dev-хуки минимум: `dump`, `click`, `key`, `state`, `shot`.

### 9.2 Фаза 1 — разделение (без смены фреймворка)

Выделить `kadr-core`, `kadr-raster`, `kadr-platform`, `kadr-media`, `kadr-ipc`, `kadr-ui` (view-model'и)
из текущего кода. GPUI-экраны становятся `kadr-ui-gpui`. Критерий готовности: `scripts/check-deps.sh` зелёный,
`just unit && just golden` зелёные, поведение не изменилось (e2e-сценарии 1–5 проходят на GPUI).

### 9.3 Фаза 2 — спайк (time-box 2 вечера, ветка `spike/<candidate>`)

Кандидаты: **Tauri 2** (редактор/настройки в webview; оверлей — `kadr-overlay`) и **iced** (пиннутая
версия). Для каждого реализовать `Shell` в объёме: `open_settings`, `show_card`, `open_editor` с тремя
инструментами (стрелка, прямоугольник, текст) поверх `EditorVm` и `kadr-raster`. Не больше.

Снять метрики раздела 2 для каждого; собрать и запустить на macOS **и** Windows (хотя бы одна из двух —
на живой машине). Заполнить таблицу в `docs/perf.md`: строки gpui / tauri / iced.

### 9.4 Правило решения

Мигрируем на кандидата, если одновременно:

1. ни одна метрика на Linux не хуже GPUI более чем на 15 %, а хоткей→оверлей и idle-RSS — не хуже вовсе;
2. спайк запустился на macOS или Windows без правок платформенного слоя фреймворка;
3. ui-тесты через хуки (6.3) для реализованных экранов проходят без изменений самих тестов —
   т. е. view-model'и действительно отделены;
4. субъективно: редактор с 4K PNG не лагает при drag (проверка человеком, одно условие на всех).

Иначе — остаёмся на GPUI CE, спайк удаляется, PR с планшетным патчем уходит в upstream gpui-ce.

### 9.5 Фаза 3 — миграция (если решили)

Экран за экраном: settings → cards → history → pin → panel → editor → studio. Старый `kadr-ui-gpui`
остаётся собираемым за фичей до конца; переключение по `--features ui-<candidate>`. Каждый экран
принимается по: `just ui` зелёный на обеих реализациях, e2e-сценарий экрана зелёный, скрин-эталоны
обновлены с явным ревью. Удаление GPUI — последний коммит фазы.

## 10. Definition of Done для всей спецификации

- `just test` и `just bench` зелёные на Linux локально и в CI; macOS/Windows — зелёные unit/golden
  и `desktop-e2e` со статусом `passed`.
- `docs/perf.md` содержит базу и результаты спайка; решение из 9.4 записано там же с датой.
- Любой экран можно протестировать без клика человека: через `kadr dev …`.
- Любой PR, меняющий растер, несёт diff golden-эталонов, проверенный человеком.
- Платформенные края из 3.10 реализованы и покрыты desktop-e2e.

## 11. Агентный режим (MCP)

kadr — «камера» для ИИ-агентов: агент сам проходит сценарий (браузер, computer use, dev-хуки kadr),
а kadr записывает, размечает и монтирует. Сценарии: скринкаст после правки прототипа, аудит интерфейса,
тест с видеодоказательством, онбординг, туториал. Отличаются только сценарием, не инструментами.

### 11.1 `kadr-mcp`

Отдельный бинарь `kadr mcp` на `rmcp` (официальный Rust SDK MCP), stdio-транспорт. Тонкий адаптер
над `kadr-ipc`: каждый инструмент — одна команда сокета; ядро не меняется. Запускается из конфига
MCP-клиента (Claude Code, Claude Desktop и др.).

Инструменты:

| Инструмент | Что делает |
|---|---|
| `capture` (`screen` \| `window` \| `area`, `monitor?`) | PNG; возвращается как image-контент и путь |
| `windows` / `monitors` | список для выбора цели |
| `record.start` (`target`, `preset?`) → `recording_id` | старт записи; лог кликов/клавиш пишется как сейчас |
| `record.marker` (`text`, `kind?: step \| expect \| note`) | метка с монотонным временем и текстом |
| `record.stop` → путь к `.kadr` | стоп; в бандле `script.json` с маркерами |
| `studio.export` (`bundle`, `preset`, `narration?: bool`, `subtitles?: bool`) → mp4 | монтаж по маркерам |
| `ocr` (`png`) | текст |
| `dev.*` | только с `KADR_DEV=1` — те же хуки, что в 6.3 |

Безопасность: MCP-сервер работает только локально через stdio; первый вызов `capture`/`record` из нового
клиента требует подтверждения в UI kadr (карточка «Агент X просит доступ к экрану», запоминается в настройках).

### 11.2 Сценарий как источник истины

`script.json` в `.kadr`-бандле:

```json
{ "version": 1,
  "steps": [
    { "t": 0.0,  "kind": "step",   "text": "Открываем настройки", "narration": "Нажимаем на шестерёнку…" },
    { "t": 4.2,  "kind": "expect", "text": "Виден список хоткеев" }
  ] }
```

- Маркеры задают тайминг шагов → зумы и главы студии (кадр зума — по кликам внутри интервала шага).
- `narration` озвучивается TTS в интервал шага; субтитры — тот же текст с тем же таймингом.
  Распознавание речи не нужно, рассинхрон невозможен, правка текста = правка видео без перезаписи.
- Сценарий редактируется в студии руками (вкладка «Сценарий») и агентом (`studio.script.update`).

### 11.3 Озвучка и субтитры (`kadr-media`)

- TTS — `Job` как ffmpeg; движок локальный по умолчанию (Piper или Kokoro), чтобы записи внутренних
  прототипов не уходили в облако; облачный TTS — опцией в настройках.
- Субтитры: встроенные (burn-in через ffmpeg) и/или отдельный `.srt`/`.vtt`.
- Тесты: `ffmpeg_args` по snapshot; синтез — за трейтом `Tts` с `FakeTts`, чтобы unit-тесты не зависели от моделей.

### 11.4 Порядок

После 3.10 и `kadr-overlay`: `record.marker` в протокол записи и `script.json` в бандл → `kadr-mcp` с
`capture`/`record.*`/`studio.export` → TTS и субтитры → подтверждение доступа в UI.

## 12. Релизы, обновления, changelog

### 12.1 Версии

SemVer. Пока `0.x`: minor — новые возможности или изменение формата `.kadr`/настроек (с миграцией),
patch — исправления. Первый публичный релиз — `0.3.0` (в `Cargo.toml` уже `0.2.0`, история не обнуляется).
`1.0.0` — когда одновременно: macOS и Windows проверены живыми пользователями (не только раннерами),
оверлей нативный на всех трёх ОС, формат `.kadr` и `settings.json` заморожены (меняются только с миграцией),
автообновление пережило хотя бы два релиза подряд. Версия одна на workspace (`[workspace.package] version`).

### 12.2 Пайплайн релиза

- Тег `v<semver>` на `main` → job `release`: сборка `dist` на трёх ОС, артефакты:
  Linux — `kadr-<ver>-x86_64-linux.tar.xz` (+ `aarch64`), AppImage позже;
  macOS — `Kadr.app` в `.dmg`, universal (arm64 + x86_64);
  Windows — `kadr-<ver>-x86_64-windows.msi` и `.zip` (portable).
  Инструмент — `cargo-dist` для сборки, установщиков и Homebrew tap; `.app`-бандл и `.dmg` — свой шаг
  (`scripts/bundle-macos.sh`), пока `cargo-dist` их не умеет.
- Подпись: macOS — Developer ID + нотаризация (Apple Developer, $99/год; без неё Gatekeeper блокирует
  и самообновление невозможно — обязательно к `1.0`, до того — `xattr -d com.apple.quarantine` в README);
  Windows — Azure Trusted Signing или без подписи с предупреждением SmartScreen до `1.0`;
  все артефакты — `minisign`-подпись ключом проекта, публичный ключ вшит в бинарь.
- CI публикует GitHub Release с телом из `CHANGELOG.md` и кладёт `latest.json` в ветку `gh-pages`
  → `https://kadr.sh/releases/latest.json`:
  ```json
  { "version": "0.3.0", "date": "2026-11-01", "notes_url": "https://kadr.sh/releases/0.3.0",
    "platforms": { "linux-x86_64": { "url": "...", "sha256": "...", "sig": "..." },
                   "macos-universal": { ... }, "windows-x86_64": { ... } } }
  ```
- После релиза CI обновляет `kadr-sh-bin` в AUR (ssh-ключ в секретах) и Homebrew tap; winget — PR в
  `microsoft/winget-pkgs` через `wingetcreate`.

### 12.3 Автообновление (`kadr-update`, крейт в `kadr-platform` или отдельный)

- **Проверка**: `kadr-tray` раз в сутки и при старте (не чаще раза в час) запрашивает `latest.json`;
  сравнение semver; результат в `cache_dir()/update.json`. Отключается в настройках. Без телеметрии:
  запрос — простой GET без идентификаторов.
- **Источник установки** определяется при старте и меняет поведение:
  `self` (архив/dmg/msi со сайта) → полное самообновление;
  `aur` / `brew` / `winget` / `flatpak` → только уведомление «Доступна 0.4.0 — обновите через <менеджер>»
  (дистрибутивы и Homebrew не терпят самообновляющихся бинарников).
  Детект: Linux — путь бинаря (`/usr/bin` → пакет, `~/.local/bin` → self) и файл-маркер от PKGBUILD;
  macOS — `brew --prefix` в пути; Windows — маркер от winget/msi.
- **Поток как у Zed**: карточка в трее/настройках «Доступна версия 0.4.0 · Что нового · Обновить».
  Нажатие → фоновая загрузка в `cache_dir()/updates/`, проверка sha256 и minisign-подписи → замена:
  Linux — атомарная замена бинарей `kadr` и `kadr-tray` (`self-replace`);
  macOS — замена `Kadr.app` рядом (переименование, не копирование внутрь), требует подписи;
  Windows — скачанный `.msi` запускается тихо после выхода приложения (`self-replace` для portable).
  Затем «Перезапустить сейчас / позже». Трей перезапускает себя сам.
- Режим «тихо в фоне» (скачать заранее, применить при следующем запуске) — настройка, по умолчанию выкл.
- Откат: предыдущий бинарь сохраняется как `kadr.prev` до первого успешного запуска новой версии;
  если новая падает дважды при старте — трей возвращает `prev` и сообщает.
- Тесты: `latest.json` парсится из фикстур; поток обновления — e2e с локальным HTTP-сервером
  и фиктивным релизом, проверяет подпись, замену и откат.

### 12.4 Changelog

- `CHANGELOG.md` в формате Keep a Changelog, генерируется `git-cliff` из conventional commits
  (`feat:`, `fix:`, `perf:`, `ui:`) в черновик, затем **редактируется в пользовательский язык** перед
  тегом: три-семь пунктов «что изменилось для человека», без имён крейтов и внутренних рефакторингов;
  скриншот или гифка для крупных фич. Два языка: `CHANGELOG.md` (en) и `docs/CHANGELOG.ru.md`.
- Тело GitHub Release = секция версии из CHANGELOG (CI вырезает автоматически).
- **Экран «Что нового»**: при первом запуске новой версии карточка с секцией
  changelog этой версии в текущем языке; markdown рендерится своими компонентами (заголовок, список,
  картинка). Также доступен из настроек. Текст берётся из бандла приложения (`data/changelog/<ver>.<lang>.md`,
  копируется CI при сборке), а не из сети — работает офлайн и не зависит от GitHub.

### 12.5 Гигиена опенсорс-репозитория

- `README.md` (en) и `docs/README.ru.md`: что это, скриншот/гифка актуального интерфейса, установка
  на три ОС (brew / winget / AUR / скачать), статус платформ, хоткеи, ссылка на спеку и CONTRIBUTING.
- `CONTRIBUTING.md`: как собрать, `just test`, conventional commits, что где лежит (ссылка на спеку).
- `CLAUDE.md` остаётся как руководство для агентов-контрибьюторов (у многих проектов это `AGENTS.md`;
  можно хранить одно и симлинкнуть), без личных путей и локальных настроек — те в `.claude/settings.local.json`
  и в `.gitignore`.
- `SECURITY.md` (куда писать об уязвимостях — `security@kadr.sh`), шаблоны issue (bug с ОС и версией,
  feature), `.github/FUNDING.yml` по желанию.
- Описание репозитория: «Screenshots and screen recording for Linux, macOS and Windows. Native,
  written in Rust.» Чужие торговые марки в публичных текстах не упоминаем. Topics: `screenshot`, `screen-recording`, `rust`, `gpui`, `wayland`, `hyprland`,
  `cross-platform`.
- Чистка: `vendor/` остаётся до принятия upstream-PR; из репозитория убрать всё, что не нужно для сборки
  и тестов (артефакты спайков, временные скрипты, личные конфиги Hyprland — остаются только шаблоны в `data/hypr`).
