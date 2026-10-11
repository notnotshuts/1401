# 1401 Linux

Приложение для проверки совместимости ПК с macOS, сборки OpenCore EFI и создания установочного USB из Linux. Интерфейс на **CustomTkinter**, аппаратные проверки — из закреплённой версии **OpCore-Simplify**.

[![Linux build](https://github.com/notnotshuts/1401/actions/workflows/linux-build.yml/badge.svg)](https://github.com/notnotshuts/1401/actions/workflows/linux-build.yml)

Это независимый Linux-форк [nullmoth/1401](https://github.com/nullmoth/1401). Целевая платформа — **x86_64 Linux**; пакеты и интеграционные проверки собираются на Ubuntu 22.04. Исходный Windows-интерфейс сохранён в `windows/`.

## Возможности

- **Проверка железа:** CPU, GPU, сеть, звук, контроллеры дисков, Bluetooth, картридеры и биометрия. Показ идентификаторов, диапазонов macOS, статусов поддержки и требований OCLP / NVIDIA-драйвера.
- **Сканирование ПК:** Hardware Sniffer собирает `Report.json` и ACPI. Можно импортировать существующий отчёт и указать отдельную папку ACPI.
- **Сборка EFI:** общий движок 1401 применяет свои политики поверх OpCore-Simplify. Интерфейс показывает SMBIOS, kexts, отключённые устройства и результат `ocvalidate`.
- **Руководство BIOS:** шаги для собранной конфигурации отображаются в приложении и сохраняются в файл.
- **Установочный USB:** выбор диска с моделью, размером и серийным номером; запрет записи на системные диски; подтверждение точным путём `/dev/...`; проверка файлов после записи.
- **Журнал и рабочие пространства:** операции выполняются в фоновых процессах, результаты сохраняются локально; журнал можно экспортировать.

## Установка

Откройте [последнюю успешную сборку Actions](https://github.com/notnotshuts/1401/actions/workflows/linux-build.yml) и скачайте артефакт **1401-linux-x86_64**. Внутри — `.deb`, переносимый `.tar.gz`, контрольные суммы и скриншоты интерфейса из CI.

Для Ubuntu 22.04+ / Debian 12+:

```bash
# В папке с распакованным артефактом
sha256sum -c SHA256SUMS
sudo apt install ./p1401-linux_*_amd64.deb
1401 gui
```

CustomTkinter и его зависимости включены в пакет с фиксированными версиями и SHA-256. Устанавливать их через `sudo pip` не нужно. Системные зависимости, включая Tk, `iasl` и инструменты записи дисков, устанавливает пакетный менеджер.

Для других дистрибутивов используйте переносимый архив; список системных зависимостей приведён в [docs/LINUX.md](docs/LINUX.md).

## Как пользоваться

1. Откройте **Проверка железа** и нажмите **Сканировать этот ПК** либо **Загрузить Report.json**. Для сканирования нужен административный доступ; установленный пакет запрашивает его через PolicyKit.
2. Выберите macOS по имени. Проверьте статусы устройств, диапазоны поддержки и замечания. Для сборки укажите папку ACPI с DSDT.
3. Откройте **Сборка EFI** и нажмите **Собрать EFI**. Сборка доступна после успешной проверки выбранной macOS и наличия ACPI. Если отчёт или версия изменены, проверку нужно повторить.
4. Прочитайте **BIOS и установка**. Сохраните резервные копии и подготовьте отдельное место для macOS.
5. Откройте **Установочный USB**, обновите список накопителей и выберите флешку. Введите её точный путь для подтверждения очистки. **Все данные на выбранном USB будут удалены.**

Если PolicyKit недоступен или запущен переносимый архив, приложение копирует команду `sudo 1401 ...` в буфер обмена для запуска в терминале. EFI можно собирать обычным пользователем.

Результаты находятся в выбранной рабочей папке: `scan/`, `compatibility.json`, `build/`, `build-result.json`, `BIOS.txt`. Отчёты и журналы автоматически никуда не отправляются.

## CLI

```bash
sudo 1401 scan /path/to/work/scan
1401 check /path/to/work/scan/Report.json --macos 24 --output /path/to/work/compatibility.json
1401 build /path/to/work/scan/Report.json /path/to/work/scan/ACPI /path/to/work/build \
  --macos 24 --result /path/to/work/build-result.json
1401 guide /path/to/work/scan/Report.json /path/to/work/build /path/to/work/BIOS.txt
1401 disks
sudo 1401 write /dev/sdX /path/to/work/build \
  --profile /path/to/work/scan/Report.json --darwin 24 --confirm-erase /dev/sdX
```

CLI принимает **Darwin major**: 23 = Sonoma 14, 24 = Sequoia 15, 25 = Tahoe 26. В интерфейсе отображаются названия macOS. Команда `check` проверяет совместимость без ACPI-компиляции и скачивания компонентов; `build` дополнительно проверяет таблицы ACPI и готовую EFI.

## Что означают результаты

| Статус устройства | Значение |
|---|---|
| Поддерживается | Правила движка допускают устройство для выбранной macOS; дополнительные kexts могут потребоваться |
| NVIDIA-драйвер | Применена политика 1401 для поддерживаемой карты NVIDIA и macOS Sequoia; используется отдельный драйвер |
| Нужен OCLP | Для выбранной версии движок указывает поддержку через OpenCore Legacy Patcher |
| Не поддерживается | Выбранная macOS вне известного диапазона либо устройство отклонено движком |
| Не проверено правилами | Для этого устройства нет оценки; это не подтверждение совместимости |

Аппаратная совместимость по таблицам **не гарантирует загрузку и работу macOS**. ACPI, разводку дисплея, USB-порты и работу драйверов необходимо проверить на конкретном ПК. Windows-only native diagnostics пока не портированы. CI проверяет запись на временном loop-образе; физическая флешка и установка macOS в рамках CI не тестируются.

Для NVIDIA после установки macOS требуется [Mac companion и драйвер upstream](https://github.com/nullmoth/nvidia-macos-driver). Linux-порт сохраняет исходные правила 1401; подробнее об устройстве движка — [HOW-IT-WORKS.md](docs/HOW-IT-WORKS.md), об upstream-версии — [исходный README](docs/UPSTREAM-README.md).

## Разработка и сборка

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install --require-hashes --no-deps -r linux/requirements.txt
python3 -m unittest discover -s tests -v
python3 -m p1401.linux gui

# На Linux: подготовить зависимости и собрать пакеты
python3 -m pip install --require-hashes --no-deps --target build/linux-vendor -r linux/requirements.txt
python3 tools/linux_release.py --output dist/linux --vendor build/linux-vendor
```

[Workflow Linux build and verification](.github/workflows/linux-build.yml) проверяет тесты, форматирование loop-образа, установку `.deb`, переносимый запуск, CustomTkinter-интерфейс под Xvfb, проверку железа, сборку EFI по закреплённому отчёту реального ПК и повторную проверку этой EFI под root.

## Лицензии

Исходный проект — [PolyForm Noncommercial 1.0.0](LICENSE). Лицензии сторонних компонентов перечислены в [NOTICE.md](NOTICE.md); CustomTkinter — MIT. Этот форк не предоставляет прав на брендинг NullMoth и не является официальной версией NullMoth Systems. Apple software не входит в пакет; образ восстановления загружается с серверов Apple.
