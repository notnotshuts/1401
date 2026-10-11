"""CustomTkinter desktop UI. All engine work runs in isolated child processes."""
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shlex
import shutil
import subprocess
import sys
import threading
from tkinter import filedialog, messagebox, StringVar

import customtkinter as ctk

from .linux import REPO
from .linux_hardware import VERSIONS

BG = '#10141c'
CARD = '#1a2130'
ACCENT = '#5a8cff'
MUTED = '#97a5bd'
GREEN = '#70d8ab'
AMBER = '#f2c66d'
RED = '#f38a93'
MACOS = {f'macOS {name}': str(n) for n, name in reversed(VERSIONS.items())}
SECTIONS = {'CPU': 'Процессор', 'GPU': 'Графика', 'Motherboard': 'Материнская плата',
            'BIOS': 'Прошивка', 'Network': 'Сеть', 'Sound': 'Звук', 'Storage Controllers': 'Контроллеры дисков',
            'Bluetooth': 'Bluetooth', 'SD Controller': 'Картридеры', 'Biometric': 'Биометрия',
            'Monitor': 'Мониторы', 'USB Controllers': 'USB', 'Input': 'Устройства ввода'}
STATUS = {'supported': ('Поддерживается', GREEN), 'driver': ('NVIDIA-драйвер', AMBER),
          'patched': ('Нужен OCLP', AMBER), 'unsupported': ('Не поддерживается', RED),
          'unknown': ('Не проверено правилами', MUTED)}


def read_json(path):
    try:
        if Path(path).stat().st_size > 16 * 1024 * 1024: return None
        value = json.loads(Path(path).read_text(encoding='utf-8'))
        return value if isinstance(value, dict) else None
    except (OSError, ValueError): return None


class Application(ctk.CTk):
    def __init__(self):
        ctk.set_appearance_mode('dark')
        ctk.set_default_color_theme('blue')
        super().__init__()
        self.title('1401 Linux — OpenCore builder')
        self.geometry('1180x820'); self.minsize(1000, 720)
        self.configure(fg_color=BG)
        self.settings_path = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / '1401/settings.json'
        settings = read_json(self.settings_path) or {}
        self.workspace = StringVar(value=settings.get('workspace', str(Path.home() / '1401-work')))
        self.report_path = StringVar(value=settings.get('report', str(self.work / 'scan/Report.json')))
        self.acpi_path = StringVar(value=settings.get('acpi', str(self.work / 'scan/ACPI')))
        self.macos = StringVar(value=settings.get('macos', 'macOS Sequoia 15'))
        if self.macos.get() not in MACOS: self.macos.set('macOS Sequoia 15')
        self.disk = StringVar(value='Выберите USB-накопитель')
        self.events = queue.Queue(); self.busy = False; self.analysis = None; self.disks = {}
        self.action_controls = []; self.pages = {}; self.nav = {}; self.current_page = 'overview'
        self.grid_columnconfigure(1, weight=1); self.grid_rowconfigure(0, weight=1)
        self._sidebar()
        self.body = ctk.CTkFrame(self, fg_color=BG, corner_radius=0)
        self.body.grid(row=0, column=1, sticky='nsew', padx=24, pady=22)
        self.body.grid_columnconfigure(0, weight=1); self.body.grid_rowconfigure(2, weight=1)
        self._header()
        for key in ('overview', 'hardware', 'build', 'guide', 'usb', 'logs'):
            frame = ctk.CTkFrame(self.body, fg_color='transparent')
            frame.grid(row=2, column=0, sticky='nsew'); self.pages[key] = frame
        self._overview(); self._hardware(); self._build(); self._guide(); self._usb(); self._logs()
        self.progress = ctk.CTkProgressBar(self.body, height=5, progress_color=ACCENT)
        self.progress.grid(row=3, column=0, sticky='ew', pady=(15, 0)); self.progress.set(0)
        self.status_label = ctk.CTkLabel(self.body, text='Готово к работе', text_color=MUTED, anchor='w')
        self.status_label.grid(row=4, column=0, sticky='ew', pady=(6, 0))
        self.show('overview'); self.update_gates()
        self.protocol('WM_DELETE_WINDOW', self.close)
        self.after(80, self.poll)
        self.report_path.trace_add('write', self.invalidate)
        self.acpi_path.trace_add('write', self.invalidate)
        self.restore()

    @property
    def work(self): return Path(self.workspace.get()).expanduser().resolve()

    @property
    def major(self): return int(MACOS[self.macos.get()])

    @property
    def build_dir(self): return self.work / 'build'

    def label(self, parent, text, size=14, color=None, **kwargs):
        return ctk.CTkLabel(parent, text=text, font=ctk.CTkFont(size=size), text_color=color or '#e8edf7', **kwargs)

    def button(self, parent, text, command, **kwargs):
        button = ctk.CTkButton(parent, text=text, command=command, height=38, **kwargs)
        self.action_controls.append(button)
        return button

    def _sidebar(self):
        side = ctk.CTkFrame(self, width=210, corner_radius=0, fg_color='#151b27')
        side.grid(row=0, column=0, sticky='ns'); side.grid_propagate(False)
        self.label(side, '1401', 38).pack(anchor='w', padx=24, pady=(28, 0))
        self.label(side, 'LINUX / OPENCORE', 11, MUTED).pack(anchor='w', padx=26, pady=(0, 32))
        for key, text in [('overview', 'Обзор'), ('hardware', 'Проверка железа'), ('build', 'Сборка EFI'),
                          ('guide', 'BIOS и установка'), ('usb', 'Установочный USB'), ('logs', 'Журнал')]:
            b = ctk.CTkButton(side, text=text, anchor='w', height=42, fg_color='transparent',
                              hover_color='#263550', command=lambda k=key: self.show(k))
            b.pack(fill='x', padx=14, pady=4); self.nav[key] = b
        self.label(side, 'Внешний вид', 11, MUTED).pack(anchor='w', padx=24, pady=(34, 8))
        self.label(side, 'Тёмная тема · CustomTkinter', 12, MUTED).pack(anchor='w', padx=24)
        self.label(side, 'Linux fork\nОтчёты остаются локально', 11, MUTED, justify='left').pack(side='bottom', anchor='w', padx=24, pady=24)

    def _header(self):
        self.title_label = self.label(self.body, 'Рабочее пространство', 25, anchor='w')
        self.title_label.grid(row=0, column=0, sticky='ew')
        row = ctk.CTkFrame(self.body, fg_color='transparent'); row.grid(row=1, column=0, sticky='ew', pady=(12, 20))
        row.grid_columnconfigure(0, weight=1)
        self.workspace_entry = ctk.CTkEntry(row, textvariable=self.workspace, height=38)
        self.workspace_entry.grid(row=0, column=0, sticky='ew', padx=(0, 10)); self.action_controls.append(self.workspace_entry)
        self.button(row, 'Выбрать папку', self.choose_workspace, width=130).grid(row=0, column=1)

    def _overview(self):
        page = self.pages['overview']; page.grid_columnconfigure((0, 1, 2), weight=1)
        self.summary_labels = {}
        for index, (key, title, value) in enumerate([('scan', 'ОТЧЁТ О ЖЕЛЕЗЕ', 'Не загружен'), ('compat', 'СОВМЕСТИМОСТЬ', 'Не проверена'), ('efi', 'СБОРКА EFI', 'Не создана')]):
            card = ctk.CTkFrame(page, fg_color=CARD); card.grid(row=0, column=index, sticky='nsew', padx=(0, 12) if index < 2 else 0)
            self.label(card, title, 11, MUTED).pack(anchor='w', padx=18, pady=(16, 8))
            label = self.label(card, value, 17, anchor='w'); label.pack(fill='x', padx=18, pady=(0, 18)); self.summary_labels[key] = label
        intro = ctk.CTkFrame(page, fg_color=CARD); intro.grid(row=1, column=0, columnspan=3, sticky='ew', pady=18)
        self.label(intro, 'От проверки ПК до установочной флешки', 23).pack(anchor='w', padx=22, pady=(20, 10))
        self.label(intro, '1. Сканируйте этот ПК или загрузите Report.json.\n2. Проверьте устройства и выберите совместимую macOS.\n3. Соберите и проверьте EFI, затем прочитайте шаги BIOS.\n4. Выберите USB-накопитель и подтвердите его очистку.', 15, MUTED, justify='left').pack(anchor='w', padx=22, pady=(0, 18))
        self.button(intro, 'Начать проверку железа', lambda: self.show('hardware'), width=230).pack(anchor='w', padx=22, pady=(0, 22))
        self.label(page, 'Поддержка по правилам не гарантирует загрузку macOS.\nМаршрутизацию дисплея, USB-порты и работу драйверов нужно проверить на самом ПК.', 13, MUTED, justify='left').grid(row=2, column=0, columnspan=3, sticky='w', pady=8)

    def _hardware(self):
        page = self.pages['hardware']; page.grid_columnconfigure(0, weight=1); page.grid_rowconfigure(4, weight=1)
        row = ctk.CTkFrame(page, fg_color='transparent'); row.grid(row=0, column=0, sticky='ew')
        self.button(row, 'Сканировать этот ПК', self.scan, width=185).pack(side='left', padx=(0, 10))
        self.button(row, 'Загрузить Report.json', self.import_report, width=185, fg_color='#33425b').pack(side='left', padx=(0, 10))
        self.button(row, 'Проверить совместимость', self.check, width=210).pack(side='left')
        paths = ctk.CTkFrame(page, fg_color=CARD); paths.grid(row=1, column=0, sticky='ew', pady=12); paths.grid_columnconfigure(1, weight=1)
        self.label(paths, 'Отчёт', 12, MUTED).grid(row=0, column=0, padx=14, pady=12)
        entry = ctk.CTkEntry(paths, textvariable=self.report_path); entry.grid(row=0, column=1, sticky='ew', padx=10); self.action_controls.append(entry)
        self.label(paths, 'ACPI', 12, MUTED).grid(row=1, column=0, padx=14, pady=(0, 12))
        entry = ctk.CTkEntry(paths, textvariable=self.acpi_path); entry.grid(row=1, column=1, sticky='ew', padx=10, pady=(0, 12)); self.action_controls.append(entry)
        self.button(paths, 'Папка…', self.choose_acpi, width=85).grid(row=1, column=2, padx=(0, 12), pady=(0, 12))
        row = ctk.CTkFrame(page, fg_color='transparent'); row.grid(row=2, column=0, sticky='ew')
        self.label(row, 'Проверять для', 13, MUTED).pack(side='left', padx=(0, 12))
        self.version_picker = ctk.CTkOptionMenu(row, values=list(MACOS), variable=self.macos, command=lambda _: self.change_version(), width=235)
        self.version_picker.pack(side='left'); self.action_controls.append(self.version_picker)
        self.hardware_summary = self.label(page, 'Загрузите отчёт, чтобы увидеть устройства и диапазоны поддержки.', 13, MUTED, anchor='w', justify='left', wraplength=820)
        self.hardware_summary.grid(row=3, column=0, sticky='ew', pady=12)
        self.hardware_rows = ctk.CTkScrollableFrame(page, fg_color=BG, label_text='Устройства и совместимость')
        self.hardware_rows.grid(row=4, column=0, sticky='nsew'); self.hardware_rows.grid_columnconfigure(0, weight=1)

    def _build(self):
        page = self.pages['build']
        self.label(page, 'Проверенная конфигурация OpenCore', 23).pack(anchor='w', pady=(0, 12))
        self.build_note = self.label(page, 'Сначала проверьте железо и укажите ACPI с таблицей DSDT.', 14, MUTED, justify='left', wraplength=820)
        self.build_note.pack(anchor='w', pady=(0, 16))
        self.build_button = self.button(page, 'Собрать EFI', self.build, width=210); self.build_button.pack(anchor='w')
        self.build_result = ctk.CTkTextbox(page, fg_color=CARD, font=ctk.CTkFont(size=14), wrap='word')
        self.build_result.pack(fill='both', expand=True, pady=18)
        self.set_text(self.build_result, 'Здесь появятся SMBIOS, версия macOS, kexts, отключённые устройства и результат ocvalidate.')

    def _guide(self):
        page = self.pages['guide']
        self.label(page, 'Шаги BIOS для этой конфигурации', 23).pack(anchor='w', pady=(0, 14))
        self.guide_button = self.button(page, 'Подготовить руководство', self.guide, width=220); self.guide_button.pack(anchor='w')
        self.guide_text = ctk.CTkTextbox(page, fg_color=CARD, wrap='word', font=ctk.CTkFont(size=14))
        self.guide_text.pack(fill='both', expand=True, pady=18)
        self.set_text(self.guide_text, 'После успешной сборки EFI здесь появятся требования прошивки и шаги установки.')

    def _usb(self):
        page = self.pages['usb']
        self.label(page, 'Создать установочную флешку', 23).pack(anchor='w', pady=(0, 12))
        self.label(page, 'Выбранный накопитель будет полностью очищен.\nСистемные диски и диски с установленной ОС утилита отклоняет.', 14, AMBER, justify='left').pack(anchor='w', pady=(0, 20))
        row = ctk.CTkFrame(page, fg_color='transparent'); row.pack(fill='x')
        self.disk_picker = ctk.CTkOptionMenu(row, variable=self.disk, values=['Выберите USB-накопитель'], width=540, command=lambda _: self.select_disk())
        self.disk_picker.pack(side='left', padx=(0, 12)); self.action_controls.append(self.disk_picker)
        self.button(row, 'Обновить', self.refresh_disks, width=120).pack(side='left')
        self.disk_note = self.label(page, 'Нажмите «Обновить», чтобы обнаружить USB-диски.', 14, MUTED, justify='left', wraplength=800)
        self.disk_note.pack(anchor='w', pady=20)
        self.write_button = self.button(page, 'Очистить и записать USB', self.write, width=240, fg_color='#ac4555', hover_color='#8f3544')
        self.write_button.pack(anchor='w')
        self.label(page, 'Образ восстановления проверяется до очистки диска.\nПосле записи файлы EFI и восстановления читаются обратно для проверки.', 13, MUTED, justify='left').pack(anchor='w', pady=24)

    def _logs(self):
        page = self.pages['logs']
        self.log = ctk.CTkTextbox(page, fg_color='#0c1017', font=ctk.CTkFont(family='monospace', size=12), wrap='word')
        self.log.pack(fill='both', expand=True)
        self.button(page, 'Сохранить журнал…', self.export_log, width=200, fg_color='#33425b').pack(anchor='w', pady=(12, 0))
        self.append_log('1401 Linux готов. Проверка железа использует правила OpCore-Simplify.\n')

    def show(self, key):
        self.current_page = key; self.pages[key].tkraise()
        for k, b in self.nav.items(): b.configure(fg_color='#263550' if k == key else 'transparent')
        self.title_label.configure(text={'overview': 'Рабочее пространство', 'hardware': 'Проверка железа', 'build': 'Сборка EFI', 'guide': 'BIOS и установка', 'usb': 'Установочный USB', 'logs': 'Журнал операций'}[key])

    def set_text(self, widget, text):
        widget.configure(state='normal'); widget.delete('1.0', 'end'); widget.insert('end', text); widget.configure(state='disabled')

    def append_log(self, text):
        self.log.configure(state='normal'); self.log.insert('end', text); self.log.see('end'); self.log.configure(state='disabled')

    def choose_workspace(self):
        folder = filedialog.askdirectory(initialdir=self.work)
        if folder:
            self.workspace.set(folder); self.report_path.set(str(self.work / 'scan/Report.json')); self.acpi_path.set(str(self.work / 'scan/ACPI'))
            self.restore()

    def choose_acpi(self):
        folder = filedialog.askdirectory()
        if folder: self.acpi_path.set(folder)

    def import_report(self):
        path = filedialog.askopenfilename(title='Hardware Sniffer Report.json', filetypes=[('JSON report', '*.json')])
        if path:
            self.report_path.set(path); self.acpi_path.set(str(Path(path).parent / 'ACPI')); self.check()

    def invalidate(self, *args):
        self.analysis = None
        self.hardware_summary.configure(text='Отчёт или ACPI изменены. Повторите проверку совместимости.')
        self.summary_labels['compat'].configure(text='Нужна проверка', text_color=MUTED)
        for widget in self.hardware_rows.winfo_children(): widget.destroy()
        self.update_gates()

    def change_version(self):
        self.invalidate()
        if Path(self.report_path.get()).is_file(): self.check()

    def analysis_current(self):
        if not self.analysis or self.analysis.get('target') != self.major: return False
        try:
            path = Path(self.report_path.get())
            if path.stat().st_size > 16 * 1024 * 1024: return False
            return hashlib.sha256(path.read_bytes()).hexdigest() == self.analysis.get('report_sha256')
        except OSError: return False

    def acpi_ready(self):
        from .scan import dsdt_in
        try: return bool(dsdt_in(self.acpi_path.get()))
        except OSError: return False

    def build_ready(self):
        from .machine_handoff import verified
        try:
            profile = verified(str(self.build_dir), self.report_path.get())
            return int(profile['macos_version'].split('.')[0]) == self.major
        except (OSError, ValueError, KeyError, RuntimeError): return False

    def update_gates(self):
        ready = self.analysis_current() and self.analysis.get('ok') and self.acpi_ready()
        self.build_button.configure(state='normal' if ready and not self.busy else 'disabled')
        built = self.build_ready()
        self.guide_button.configure(state='normal' if built and not self.busy else 'disabled')
        disk = self.disks.get(self.disk.get())
        self.write_button.configure(state='normal' if built and disk and disk.get('eligible') and disk.get('SerialNumber') and not self.busy else 'disabled')
        self.summary_labels['scan'].configure(text='Загружен' if Path(self.report_path.get()).is_file() else 'Не загружен')
        self.summary_labels['efi'].configure(text='Проверена' if built else 'Не создана', text_color=GREEN if built else MUTED)
        if ready:
            self.build_note.configure(text=f'{self.macos.get()} • ACPI: DSDT найдена\nРезультат будет сохранён в {self.build_dir}')
        else:
            self.build_note.configure(text='Для сборки нужна актуальная успешная проверка выбранной macOS и папка ACPI с DSDT.')

    def command(self, args, privileged=False):
        if privileged and os.geteuid() != 0:
            if shutil.which('pkexec') and Path('/usr/bin/1401').is_file():
                return ['pkexec', '/usr/bin/1401', *args]
            text = shlex.join(['sudo', '1401', *args])
            self.clipboard_clear(); self.clipboard_append(text)
            messagebox.showinfo('Нужны права администратора', 'Команда скопирована в буфер обмена. Выполните её в терминале:\n\n' + text)
            return None
        return [sys.executable, '-u', '-m', 'p1401.linux', *args]

    def start(self, args, title, callback=None, privileged=False):
        if self.busy: return
        command = self.command(args, privileged)
        if command is None: return
        self.work.mkdir(parents=True, exist_ok=True)
        self.busy = True; self.progress.configure(mode='indeterminate'); self.progress.start()
        self.status_label.configure(text=title, text_color=MUTED)
        self.append_log('\n' + title + '\n' + shlex.join(command) + '\n')
        for control in self.action_controls: control.configure(state='disabled')
        def worker():
            returncode = 1
            try:
                p = subprocess.Popen(command, cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                for line in p.stdout: self.events.put(('log', line))
                returncode = p.wait()
            except Exception as error: self.events.put(('log', str(error) + '\n'))
            self.events.put(('done', (returncode, callback)))
        threading.Thread(target=worker, daemon=True).start()

    def poll(self):
        while not self.events.empty():
            kind, value = self.events.get()
            if kind == 'log':
                self.append_log(value)
                match = re.search(r'Download (\d+)/(\d+)', value)
                if match and int(match[2]):
                    self.progress.stop(); self.progress.configure(mode='determinate'); self.progress.set(int(match[1]) / int(match[2]))
            else:
                code, callback = value
                self.busy = False; self.progress.stop(); self.progress.configure(mode='determinate'); self.progress.set(1 if code == 0 else 0)
                for control in self.action_controls: control.configure(state='normal')
                self.status_label.configure(text='Операция завершена' if code == 0 else f'Операция остановлена (код {code}). Причина — в журнале.', text_color=GREEN if code == 0 else RED)
                if callback:
                    try: callback(code)
                    except Exception as error:
                        self.append_log(f'Не удалось прочитать результат: {error}\n')
                        self.status_label.configure(text='Не удалось прочитать результат. См. журнал.', text_color=RED)
                self.update_gates(); self.save_settings()
        self.after(80, self.poll)

    def scan(self):
        path = self.work / 'scan'
        self.report_path.set(str(path / 'Report.json')); self.acpi_path.set(str(path / 'ACPI'))
        self.start(['scan', str(path)], 'Сканирование оборудования и ACPI…', lambda code: self.check() if code == 0 else None, privileged=True)

    def check(self):
        path = self.work / 'compatibility.json'
        # A previous result can never stand in for a failed new check.
        if path.exists(): path.unlink()
        self.start(['check', self.report_path.get(), '--macos', str(self.major), '--output', str(path)],
                   'Проверка совместимости оборудования…', lambda _: self.load_analysis(path))

    def load_analysis(self, path):
        self.analysis = read_json(path)
        if not self.analysis: raise RuntimeError('Проверка не сохранила результат.')
        self.render_hardware(); self.update_gates()

    def render_hardware(self):
        data = self.analysis
        for widget in self.hardware_rows.winfo_children(): widget.destroy()
        versions = ', '.join(VERSIONS[n] for n in data.get('native_versions', [])) or 'нет'
        patched = ', '.join(VERSIONS[n] for n in data.get('oclp_versions', []))
        summary = f'Поддерживаемые версии: {versions}.' + (f' С OCLP: {patched}.' if patched else '')
        messages = data.get('blockers', []) + data.get('warnings', [])
        if messages:
            summary += '\n' + '\n'.join(str(message)[:240] for message in messages[:2])
            if len(messages) > 2 or any(len(str(message)) > 240 for message in messages):
                summary += '\nПолные замечания доступны в журнале и compatibility.json.'
        summary += '\nACPI: ' + ('DSDT найдена.' if self.acpi_ready() else 'DSDT не найдена — для сборки требуется полный ACPI dump.')
        self.hardware_summary.configure(text=summary, text_color=MUTED if data.get('ok') else AMBER)
        self.summary_labels['compat'].configure(text='Подходит выбранная macOS' if data.get('ok') else 'Есть ограничения', text_color=GREEN if data.get('ok') else AMBER)
        for index, device in enumerate(data.get('devices', [])):
            card = ctk.CTkFrame(self.hardware_rows, fg_color=CARD)
            card.grid(row=index, column=0, sticky='ew', pady=5); card.grid_columnconfigure(0, weight=1)
            title, color = STATUS[device['status']]
            self.label(card, SECTIONS.get(device['section'], device['section']), 11, MUTED).grid(row=0, column=0, sticky='w', padx=16, pady=(12, 0))
            self.label(card, device['name'], 15, anchor='w', wraplength=560, justify='left').grid(row=1, column=0, sticky='ew', padx=16, pady=(4, 8))
            self.label(card, title, 12, color).grid(row=1, column=1, padx=16)
            props = device['details']
            details = ' • '.join(f'{k}: {v}' for k, v in props.items() if k in ('Device ID', 'Codename', 'Core Count', 'Chipset', 'Firmware Type', 'SIMD Features', 'Bus Type', 'Connector Type', 'Resolution', 'Secure Boot', 'Platform', 'CPU Count'))
            def range_text(value):
                if not value or not all(value): return 'нет'
                return f'Darwin {value[1]} — {value[0]}'
            if device['native'] is not None: details += '\nПоддержка: ' + range_text(device['native'])
            if device['oclp']: details += ' • OCLP: ' + range_text(device['oclp'])
            self.label(card, details or 'Сведения из отчёта; совместимость не оценивается этой группой правил.', 12, MUTED, wraplength=730, justify='left', anchor='w').grid(row=2, column=0, columnspan=2, sticky='ew', padx=16, pady=(0, 12))

    def build(self):
        if not self.analysis_current() or not self.analysis.get('ok') or not self.acpi_ready():
            self.invalidate(); messagebox.showinfo('Нужна проверка', 'Отчёт изменился или нет ACPI. Проверьте железо снова.'); return
        result = self.work / 'build-result.json'
        if result.exists(): result.unlink()
        self.start(['build', self.report_path.get(), self.acpi_path.get(), str(self.build_dir), '--macos', str(self.major), '--result', str(result)],
                   'Сборка и проверка OpenCore EFI…', lambda _: self.load_build(result))

    def load_build(self, path):
        result = read_json(path)
        if not result: raise RuntimeError('Нет результата сборки; см. журнал.')
        lines = ['EFI проверена' if result.get('ok') else 'Сборка остановлена',
                 f'SMBIOS: {result.get("smbios") or "—"}', f'Darwin: {result.get("macos_version") or "—"}',
                 f'OCLP: {"требуется" if result.get("needs_oclp") else "не требуется"}',
                 'Kexts: ' + ', '.join(result.get('kexts', [])),
                 'Отключённые устройства: ' + json.dumps(result.get('disabled_devices', {}), ensure_ascii=False),
                 'Проверка: ' + json.dumps(result.get('validation', {}), ensure_ascii=False, indent=2)]
        lines.extend(result.get('notices', []))
        if result.get('error'): lines.append(result['error'])
        self.set_text(self.build_result, '\n\n'.join(lines))

    def guide(self):
        path = self.work / 'BIOS.txt'
        self.start(['guide', self.report_path.get(), str(self.build_dir), str(path)], 'Подготовка шагов BIOS…',
                   lambda code: self.set_text(self.guide_text, path.read_text(encoding='utf-8')) if code == 0 else None)

    def refresh_disks(self):
        self.start(['disks'], 'Поиск USB-накопителей…', self.load_disks)

    def load_disks(self, code):
        self.disks.clear()
        if code == 0:
            # Disk JSON is also in the log; read-only discovery is cheap and has no engine package conflict.
            from .linux_usb import list_disks, eligible
            for disk in list_disks():
                if disk['BusType'] != 'USB': continue
                ok, reasons = eligible(disk)
                if not disk.get('SerialNumber'): ok = False; reasons.append('нет стабильного серийного номера')
                disk.update(eligible=ok, reasons=reasons)
                label = f'{disk["Path"]} · {disk["FriendlyName"] or "USB"} · {disk["Size"] / 2**30:.1f} GiB' + ('' if ok else ' · недоступен')
                self.disks[label] = disk
        values = list(self.disks) or ['USB-накопители не обнаружены']
        self.disk_picker.configure(values=values); self.disk.set(values[0]); self.select_disk()

    def select_disk(self):
        disk = self.disks.get(self.disk.get())
        text = 'USB-накопители не обнаружены.'
        if disk:
            text = f'Диск: {disk["Path"]}\nМодель: {disk["FriendlyName"]}\nСерийный номер: {disk["SerialNumber"] or "неизвестен"}\n'
            text += 'Готов к выбору. Все данные будут удалены.' if disk['eligible'] else 'Запись запрещена: ' + '; '.join(disk['reasons'])
        self.disk_note.configure(text=text); self.update_gates()

    def write(self):
        disk = self.disks.get(self.disk.get())
        if not disk or not disk['eligible'] or not self.build_ready(): self.update_gates(); return
        dialog = ctk.CTkInputDialog(text=f'Будут удалены ВСЕ данные на {disk["Path"]}\n{disk["FriendlyName"]} · {disk["Size"] / 2**30:.1f} GiB\n\nВведите путь диска для подтверждения:', title='Подтвердить очистку USB')
        if dialog.get_input() != disk['Path']: return
        self.start(['write', disk['Path'], str(self.build_dir), '--profile', self.report_path.get(), '--darwin', str(self.major), '--confirm-erase', disk['Path']],
                   'Загрузка восстановления и запись USB…', privileged=True)

    def restore(self):
        path = self.work / 'compatibility.json'
        self.analysis = read_json(path)
        if self.analysis_current(): self.render_hardware()
        else: self.analysis = None
        if read_json(self.work / 'build-result.json'): self.load_build(self.work / 'build-result.json')
        if (self.work / 'BIOS.txt').is_file(): self.set_text(self.guide_text, (self.work / 'BIOS.txt').read_text(encoding='utf-8'))
        self.update_gates()

    def save_settings(self):
        self.settings_path.parent.mkdir(parents=True, exist_ok=True)
        self.settings_path.write_text(json.dumps({'workspace': str(self.work), 'report': self.report_path.get(), 'acpi': self.acpi_path.get(), 'macos': self.macos.get()}, ensure_ascii=False), encoding='utf-8')

    def export_log(self):
        path = filedialog.asksaveasfilename(defaultextension='.txt', initialfile='1401-log.txt')
        if path: Path(path).write_text(self.log.get('1.0', 'end'), encoding='utf-8')

    def close(self):
        if self.busy:
            messagebox.showinfo('Операция выполняется', 'Дождитесь завершения операции перед закрытием окна.'); return
        self.save_settings(); self.destroy()
