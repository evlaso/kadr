# Командная строка

`kadr --help` показывает список команд. Если kadr уже работает, последующие команды съёмки, редактирования и настроек передаются этому экземпляру, а не запускают ещё один процесс интерфейса.

```text
  kadr capture                                   every mode on one bar
  kadr shot [smart|region|windows|fullscreen|all|timer|previous]
                                                 take a screenshot (smart by default; all: every screen at once;
                                                 previous: the last area)
  kadr shot scroll ["X,Y WxH"]                    scrolling screenshot (again to finish)
  kadr open <file>                               show the thumbnail for an existing image (a .kadr opens its project)
  kadr pin <file>                                pin an image on top of windows

  kadr record                                    the recording bar (or stop a running recording)
  kadr record region|fullscreen [--mic] [--audio] [--webcam] [--keys] [--studio]
                                                 start recording at once (again to stop)
  kadr record area "X,Y WxH"                     the recording bar for a given area
  kadr record previous                           the recording bar for the last area picked
  kadr record stop                               stop recording
  kadr studio <video|project.kadr> [--render]    studio: zooms, background, clicks and keys (--render exports at once)
  kadr edit <image|project.kadr>                 editor: arrows, text, highlighter, blur…
  kadr trim <video> [--from S --to S]            trim a recording (no flags: the trim window)
  kadr gif <video>                               save a recording as GIF (720px, 15 fps)
  kadr history                                   history of screenshots and recordings
  kadr settings                                  settings
  kadr update                                    look for a newer release now
  kadr tray                                      tray icon (for autostart)
  kadr dev <command>                             test hooks of a kadr started with KADR_DEV=1
                                                 (windows, dump, find, click, drag, key, type, state, shot, mem)
  kadr text [image]                              recognize text (in an area or an image) and copy it
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
  kadr --version                                 the version
```

## Скриншоты

`kadr capture` открывает панель со всеми режимами съёмки. `kadr shot` по умолчанию запускает умный выбор; явно можно выбрать область, окна, экран, все экраны, таймер или прошлую область. `kadr shot scroll` начинает снимок с прокруткой и завершает его при повторном вызове. `kadr open <file>` показывает существующую картинку на карточке; `kadr pin <file>` закрепляет её поверх окон. Подробнее — в [съёмке](capture.md).

## Запись

`kadr record` открывает панель записи или останавливает текущую запись. Варианты region и fullscreen начинают сразу; можно включить микрофон, звук системы, камеру, показ сочетаний и режим студии. Варианты area и previous открывают панель для заданной или прошлой области. `kadr record stop` явно останавливает запись. Подробнее — в [записи](record.md); лог кликов и клавиш через Hyprland работает только на Linux.

## Редактирование

`kadr edit <image|project.kadr>` открывает [редактор](editor.md); `kadr studio <video|project.kadr>` — [студию](studio.md), а флаг render сразу запускает экспорт. `kadr open <file>` открывает проект в нужном редакторе. `kadr trim <video>` открывает обрезку или обрезает сразу по времени в секундах; для начала нужно задать и конец. `kadr gif <video>` делает GIF размером 720px с частотой 15 кадров в секунду. Как сохранить правку — в [файлах проектов](projects.md).

## Остальные команды

`kadr history` открывает [историю](history.md), `kadr settings` — [настройки](settings.md). `kadr text [image]` распознаёт текст на картинке или предлагает выбрать область и копирует результат. `kadr update` проверяет новый релиз сразу; `kadr tray` запускает трей для автозапуска. `kadr install-desktop` регистрирует файлы проектов на Linux. `kadr --version` печатает версию. Команда для разработки предназначена для тестовых хуков, а не для обычной съёмки.
