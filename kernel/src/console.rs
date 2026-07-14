use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use browser_core::{HtmlTokenizer, Url};
use core::{fmt, ptr};

mod inter {
    include!(concat!(env!("OUT_DIR"), "/inter_atlas.rs"));
}

static mut WRITER: Option<Writer> = None;

const GLASS: [u8; 3] = [12, 28, 55];
const GLASS_2: [u8; 3] = [18, 39, 72];
const GLASS_3: [u8; 3] = [25, 49, 86];
const BORDER: [u8; 3] = [55, 83, 124];
const BORDER_SOFT: [u8; 3] = [36, 61, 99];
const TEXT: [u8; 3] = [239, 245, 255];
const MUTED: [u8; 3] = [152, 171, 203];
const CYAN: [u8; 3] = [57, 206, 255];
const BLUE: [u8; 3] = [82, 124, 255];
const VIOLET: [u8; 3] = [147, 94, 255];
const MINT: [u8; 3] = [52, 211, 153];
const YELLOW: [u8; 3] = [255, 190, 54];
const RED: [u8; 3] = [255, 92, 108];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum App {
    Home,
    Files,
    Terminal,
    Ai,
    Browser,
    Settings,
    Editor,
    Packages,
}

#[derive(Clone, Copy)]
pub enum GuardianUiState {
    Ready,
    VerifiedHistory,
    RecoveryRequired,
}

pub struct Writer {
    buffer: *mut u8,
    info: FrameBufferInfo,
    x: usize,
    y: usize,
    content_left: usize,
    content_top: usize,
    content_right: usize,
    content_bottom: usize,
    foreground: [u8; 3],
    app: App,
    cursor_x: usize,
    cursor_y: usize,
    cursor_visible: bool,
}

unsafe impl Send for Writer {}

pub fn init(framebuffer: &'static mut FrameBuffer) {
    let info = framebuffer.info();
    unsafe {
        WRITER = Some(Writer {
            buffer: framebuffer.buffer_mut().as_mut_ptr(),
            info,
            x: 0,
            y: 0,
            content_left: 0,
            content_top: 0,
            content_right: 0,
            content_bottom: 0,
            foreground: TEXT,
            app: App::Home,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
        });
    }
    show_home();
}

fn with_writer(action: impl FnOnce(&mut Writer)) {
    unsafe {
        if let Some(writer) = &mut *ptr::addr_of_mut!(WRITER) {
            action(writer);
        }
    }
}

pub fn dimensions() -> (usize, usize) {
    let mut result = (1280, 720);
    with_writer(|w| result = (w.info.width, w.info.height));
    result
}

pub fn move_pointer(x: usize, y: usize) {
    with_writer(|w| w.pointer(x, y));
}

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    crate::serial::write_fmt(args);
    with_writer(|writer| {
        let _ = writer.write_fmt(args);
    });
}

pub fn show_home() {
    with_writer(|w| w.home());
}

pub fn show_files() {
    with_writer(|w| {
        w.begin(App::Files);
        let margin = w.info.width / 24;
        let task_y = w.taskbar_y();
        w.files_window(margin, 36, w.info.width - margin * 2, task_y - 54, false);
        w.taskbar(App::Files);
    });
}

pub fn file_item(path: &str, index: usize) {
    with_writer(|w| {
        let columns = ((w.content_right - w.content_left) / 170).max(1);
        let col = index % columns;
        let row = index / columns;
        let x = w.content_left + col * 170;
        let y = w.content_top + row * 92;
        w.file_tile(x, y, path, index == 0);
    });
}

pub fn finish_files() {
    with_writer(|w| w.taskbar(App::Files));
}

pub fn show_terminal() {
    with_writer(|w| {
        w.begin(App::Terminal);
        let margin = w.info.width / 15;
        let task_y = w.taskbar_y();
        w.window(
            margin,
            48,
            w.info.width - margin * 2,
            task_y - 70,
            "Термінал",
            "Командне середовище Nova",
        );
        w.fill_rect(margin + 22, 112, w.info.width - margin * 2 - 44, 1, BORDER);
        w.content_left = margin + 28;
        w.content_right = w.info.width - margin - 28;
        w.content_top = 134;
        w.content_bottom = task_y - 36;
        w.x = w.content_left;
        w.y = w.content_top;
        w.taskbar(App::Terminal);
    });
}

pub fn show_ai(guardian: GuardianUiState) {
    with_writer(|w| {
        w.begin(App::Ai);
        let width = (w.info.width * 3 / 5).max(620);
        let x = (w.info.width - width) / 2;
        let task_y = w.taskbar_y();
        w.window(x, 42, width, task_y - 62, "Nova AI", "Системний оператор");
        w.ai_hero(x + 28, 118, width - 56);
        w.guardian_status(x + 28, 246, width - 56, guardian);
        w.content_left = x + 42;
        w.content_right = x + width - 42;
        w.content_top = 322;
        w.content_bottom = task_y - 50;
        w.x = w.content_left;
        w.y = w.content_top;
        w.taskbar(App::Ai);
    });
}

pub fn show_browser() {
    with_writer(|w| {
        w.begin(App::Browser);
        let margin = w.info.width / 24;
        let task_y = w.taskbar_y();
        let width = w.info.width - margin * 2;
        w.window(
            margin,
            36,
            width,
            task_y - 54,
            "Браузер Nova",
            "Приватність за задумом",
        );
        let toolbar_y = 94;
        w.text(margin + 24, toolbar_y + 15, "<   >   R", MUTED);
        w.round_rect_blend(
            margin + 118,
            toolbar_y + 5,
            width - 260,
            38,
            12,
            [5, 18, 40],
            225,
        );
        w.fill_rect(margin + 135, toolbar_y + 20, 7, 7, MINT);
        w.text(margin + 153, toolbar_y + 14, "nova://start", TEXT);
        w.round_rect(margin + width - 122, toolbar_y + 7, 92, 34, 10, GLASS_3);
        w.text(margin + width - 101, toolbar_y + 15, "Меню", MUTED);
        w.browser_start(
            margin + 34,
            toolbar_y + 66,
            width - 68,
            task_y - toolbar_y - 100,
        );
        w.taskbar(App::Browser);
    });
}

pub fn show_settings() {
    with_writer(|w| {
        w.begin(App::Settings);
        let width = (w.info.width * 2 / 3).max(700);
        let x = (w.info.width - width) / 2;
        let task_y = w.taskbar_y();
        w.window(
            x,
            42,
            width,
            task_y - 62,
            "Центр керування",
            "Швидкі параметри системи",
        );
        w.quick_settings(x + 30, 122, width - 60, task_y - 190);
        w.taskbar(App::Settings);
    });
}

pub fn show_editor() {
    with_writer(|w| {
        w.begin(App::Editor);
        let margin = w.info.width / 18;
        let task_y = w.taskbar_y();
        let width = w.info.width - margin * 2;
        w.window(
            margin,
            38,
            width,
            task_y - 58,
            "Редактор Nova",
            "Нова-нотатка.txt",
        );
        w.fill_rect(margin, 93, width, 44, GLASS_2);
        w.text(
            margin + 22,
            107,
            "Файл   Редагування   Збірка   Запустити",
            MUTED,
        );
        w.round_rect(margin + width - 170, 101, 142, 28, 8, BLUE);
        w.text(margin + width - 148, 108, "Зберегти: Enter", TEXT);
        w.fill_rect(margin + 54, 158, 1, task_y - 218, BORDER);
        for row in 1..=12 {
            w.text(
                margin + 20,
                151 + row * 24,
                if row < 10 { "  " } else { "" },
                MUTED,
            );
        }
        w.content_left = margin + 74;
        w.content_right = margin + width - 28;
        w.content_top = 168;
        w.content_bottom = task_y - 36;
        w.x = w.content_left;
        w.y = w.content_top;
        w.text(margin + 18, task_y - 26, "UTF-8   Rust   Рядок 1", MUTED);
        w.taskbar(App::Editor);
    });
}

pub fn show_packages() {
    with_writer(|w| {
        w.begin(App::Packages);
        let width = (w.info.width * 3 / 4).max(760);
        let x = (w.info.width - width) / 2;
        let task_y = w.taskbar_y();
        w.window(
            x,
            42,
            width,
            task_y - 62,
            "Програми Nova",
            "Пакети та оновлення",
        );
        w.heading(x + 30, 116, "Встановлені системні програми", TEXT);
        let names = [
            "Файли",
            "Термінал",
            "Редактор Nova",
            "Браузер Nova",
            "Nova AI",
        ];
        for (index, name) in names.iter().enumerate() {
            let y = 154 + index * 58;
            w.round_rect(x + 28, y, width - 56, 46, 10, GLASS_2);
            w.fill_circle(x + 52, y + 23, 8, if index == 3 { CYAN } else { VIOLET });
            w.text(x + 72, y + 14, name, TEXT);
            w.text(x + width - 138, y + 14, "Встановлено", MINT);
        }
        w.text(x + 30, task_y - 34, "F8 — оновити список пакетів", MUTED);
        w.taskbar(App::Packages);
    });
}

pub fn hit_test(x: usize, y: usize) -> Option<App> {
    let mut result = None;
    with_writer(|w| {
        let task_y = w.taskbar_y();
        if y < task_y || y > task_y + 58 {
            if w.app == App::Home {
                let (files_x, files_y, files_w, files_h, ai_x, ai_w) = w.home_layout();
                if (files_x..files_x + files_w).contains(&x)
                    && (files_y..files_y + files_h).contains(&y)
                {
                    result = Some(App::Files);
                } else if (ai_x..ai_x + ai_w).contains(&x) && (30..390).contains(&y) {
                    result = Some(App::Ai);
                }
            }
            return;
        }
        let center = w.info.width / 2;
        result = if x < 180 {
            Some(App::Home)
        } else if (center.saturating_sub(245)..center.saturating_sub(140)).contains(&x) {
            Some(App::Files)
        } else if (center.saturating_sub(130)..center.saturating_sub(20)).contains(&x) {
            Some(App::Browser)
        } else if (center.saturating_sub(10)..center + 110).contains(&x) {
            Some(App::Terminal)
        } else if (center + 120..center + 240).contains(&x) {
            Some(App::Ai)
        } else if x > w.info.width.saturating_sub(260) {
            Some(App::Settings)
        } else {
            None
        };
    });
    result
}

impl Writer {
    fn begin(&mut self, app: App) {
        self.restore_pointer();
        self.app = app;
        self.wallpaper();
        self.foreground = TEXT;
    }

    fn home(&mut self) {
        self.begin(App::Home);
        let (fx, fy, fw, fh, ax, aw) = self.home_layout();
        self.files_window(fx, fy, fw, fh, true);
        let ai_h = fh * 58 / 100;
        self.ai_panel(ax, 30, aw, ai_h);
        self.quick_settings(ax, 30 + ai_h + 14, aw, fh - ai_h - 14);
        self.taskbar(App::Home);
    }

    fn home_layout(&self) -> (usize, usize, usize, usize, usize, usize) {
        let margin = self.info.width / 30;
        let gap = 18;
        let right = (self.info.width * 24 / 100).max(285);
        let files_w = self.info.width.saturating_sub(margin * 2 + right + gap);
        let height = self.taskbar_y().saturating_sub(62);
        (margin, 42, files_w, height, margin + files_w + gap, right)
    }

    fn taskbar_y(&self) -> usize {
        self.info.height.saturating_sub(70)
    }

    fn wallpaper(&mut self) {
        let width = self.info.width.max(1);
        let height = self.info.height.max(1);
        for y in 0..height {
            let horizon = height * 3 / 5;
            for x in 0..width {
                let mut r = 3u8;
                let mut g = (7 + y * 8 / height) as u8;
                let mut b = (23 + y * 15 / height) as u8;
                let blue_center = width * 68 / 100 + y / 5;
                let blue_dist = x.abs_diff(blue_center);
                if blue_dist < 115 {
                    let glow = ((115 - blue_dist) * 55 / 115) as u8;
                    g = g.saturating_add(glow / 2);
                    b = b.saturating_add(glow);
                }
                let violet_center = width * 88 / 100 - y / 7;
                let violet_dist = x.abs_diff(violet_center);
                if violet_dist < 150 {
                    let glow = ((150 - violet_dist) * 48 / 150) as u8;
                    r = r.saturating_add(glow / 2);
                    b = b.saturating_add(glow);
                }
                if y > horizon {
                    let wave = (x + (y - horizon) * 3) % 260;
                    if wave < 46 {
                        b = b.saturating_add(((46 - wave) / 4) as u8);
                        g = g.saturating_add(((46 - wave) / 8) as u8);
                    }
                }
                self.pixel(x, y, [r, g, b]);
            }
        }
        self.fill_rect(0, 0, width, 2, CYAN);
    }

    fn window(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        title: &str,
        subtitle: &str,
    ) {
        self.round_rect_blend(x + 12, y + 18, width + 4, height + 4, 24, [0, 0, 7], 92);
        self.round_rect_blend(x + 6, y + 9, width + 3, height + 3, 21, [0, 0, 10], 142);
        self.round_rect_blend(x, y, width, height, 18, GLASS, 238);
        self.stroke_rect(x, y, width, height, 18, BORDER_SOFT);
        self.fill_rect(x + 20, y, width.saturating_sub(40), 1, [82, 116, 164]);
        self.fill_rect(x, y + 54, width, 1, BORDER);
        self.fill_circle(x + 24, y + 26, 6, VIOLET);
        self.fill_circle(x + 24, y + 26, 2, CYAN);
        self.heading(x + 40, y + 10, title, TEXT);
        self.text(x + 40, y + 32, subtitle, MUTED);
        self.window_controls(x + width - 105, y + 20);
    }

    fn files_window(&mut self, x: usize, y: usize, width: usize, height: usize, preview: bool) {
        self.window(x, y, width, height, "Файли", "Проєкти");
        let toolbar_y = y + 55;
        self.fill_rect(x, toolbar_y, width, 52, GLASS_2);
        self.text(x + 20, toolbar_y + 18, "<   >   ^", MUTED);
        self.round_rect(
            x + 115,
            toolbar_y + 10,
            width.saturating_sub(310),
            32,
            8,
            [8, 22, 44],
        );
        self.text(x + 132, toolbar_y + 17, "Домівка  >  Проєкти", TEXT);
        self.round_rect(x + width - 175, toolbar_y + 10, 150, 32, 8, GLASS_3);
        self.text(x + width - 155, toolbar_y + 17, "Пошук", MUTED);

        let side = if width > 800 { 180 } else { 150 };
        let body_y = toolbar_y + 53;
        self.fill_rect(
            x,
            body_y,
            side,
            height.saturating_sub(body_y - y),
            [10, 25, 49],
        );
        self.nav_row(x + 14, body_y + 15, side - 28, "Домівка", true);
        self.nav_row(x + 14, body_y + 55, side - 28, "Галерея", false);
        self.nav_row(x + 14, body_y + 95, side - 28, "Завантаження", false);
        self.nav_row(x + 14, body_y + 135, side - 28, "Документи", false);
        self.text(x + 24, body_y + 190, "РОЗТАШУВАННЯ", MUTED);
        self.text(x + 24, body_y + 220, "Nova OS", TEXT);
        self.text(x + 24, body_y + 250, "Спільне", TEXT);

        let grid_x = x + side + 28;
        self.content_left = grid_x;
        self.content_top = body_y + 62;
        self.content_right = x + width - 26;
        self.content_bottom = y + height - 32;
        self.heading(grid_x, body_y + 19, "Останні файли", TEXT);
        self.round_rect(x + width - 112, body_y + 16, 82, 25, 12, GLASS_3);
        self.text(
            x + width - 96,
            body_y + 21,
            if preview {
                "6 об’єктів"
            } else {
                "Вміст"
            },
            MUTED,
        );
        if preview {
            self.file_tile(grid_x, body_y + 62, "Проєкти", false);
            self.file_tile(grid_x + 160, body_y + 62, "Nova OS", true);
            self.file_tile(grid_x + 320, body_y + 62, "Ресурси", false);
            if width > 760 {
                self.file_tile(grid_x + 480, body_y + 62, "Студія", false);
            }
            self.file_tile(grid_x, body_y + 159, "README.txt", false);
            self.file_tile(grid_x + 160, body_y + 159, "Roadmap.md", false);
        }
    }

    fn ai_panel(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.window(
            x,
            y,
            width,
            height,
            "Nova AI",
            "Автономний оператор | готовий",
        );
        self.round_rect(x + 20, y + 72, width - 40, 74, 12, GLASS_2);
        self.round_rect_blend(x + 30, y + 87, 40, 40, 20, VIOLET, 242);
        self.fill_circle(x + 50, y + 107, 8, [39, 42, 101]);
        self.fill_circle(x + 50, y + 107, 3, CYAN);
        self.text(x + 82, y + 84, "Чим допомогти?", TEXT);
        self.fill_rect(x + 82, y + 111, 7, 7, MINT);
        self.text(x + 96, y + 105, "Система доступна", MINT);
        self.round_rect(x + 20, y + 160, width - 40, 78, 12, GLASS_2);
        self.text(x + 34, y + 173, "ВИКОНУЄТЬСЯ", MUTED);
        self.text(x + 34, y + 198, "Оптимізація стільниці", TEXT);
        self.fill_rect(x + 34, y + 222, width - 100, 4, [35, 54, 83]);
        self.fill_rect(x + 34, y + 222, (width - 100) * 7 / 10, 4, BLUE);
        if height > 300 {
            self.round_rect_blend(x + 20, y + height - 62, width - 40, 42, 12, GLASS_3, 244);
            self.stroke_rect(x + 20, y + height - 62, width - 40, 42, 12, BORDER_SOFT);
            self.text(x + 35, y + height - 49, "Напишіть завдання...", MUTED);
            self.round_rect(x + width - 58, y + height - 55, 28, 28, 14, BLUE);
            self.text(x + width - 49, y + height - 50, ">", TEXT);
        }
    }

    fn ai_hero(&mut self, x: usize, y: usize, width: usize) {
        self.round_rect_blend(x + 4, y + 7, width, 118, 18, [0, 0, 10], 105);
        self.round_rect_blend(x, y, width, 118, 16, GLASS_2, 244);
        self.stroke_rect(x, y, width, 118, 16, BORDER_SOFT);
        self.round_rect_blend(x + 22, y + 22, 62, 62, 31, VIOLET, 244);
        self.fill_circle(x + 53, y + 53, 13, [39, 42, 101]);
        self.fill_circle(x + 53, y + 53, 5, CYAN);
        self.heading(x + 108, y + 20, "Що потрібно зробити?", TEXT);
        self.text(
            x + 108,
            y + 50,
            "Nova працює з файлами, програмами й налаштуваннями системи.",
            MUTED,
        );
        self.fill_circle(x + 112, y + 87, 4, MINT);
        self.text(
            x + 126,
            y + 78,
            "Локальний режим | дані залишаються на пристрої",
            MINT,
        );
    }

    fn guardian_status(&mut self, x: usize, y: usize, width: usize, state: GuardianUiState) {
        self.round_rect_blend(x, y, width, 58, 14, [7, 26, 47], 244);
        self.stroke_rect(x, y, width, 58, 14, [36, 93, 119]);
        self.fill_circle(x + 30, y + 29, 16, [18, 83, 89]);
        self.stroke_box(x + 24, y + 18, 12, 16, MINT);
        self.fill_rect(x + 27, y + 24, 6, 7, MINT);
        self.text(x + 58, y + 11, "Nova Guardian активний", TEXT);
        self.text(
            x + 58,
            y + 34,
            "Журнал | перевірка | безпечний відкат",
            MUTED,
        );
        let (label, color) = match state {
            GuardianUiState::Ready => ("Захищено", MINT),
            GuardianUiState::VerifiedHistory => ("Перевірено", MINT),
            GuardianUiState::RecoveryRequired => ("Recovery", YELLOW),
        };
        self.round_rect(x + width - 124, y + 14, 102, 30, 10, [13, 65, 70]);
        self.fill_circle(x + width - 105, y + 29, 4, color);
        self.text(x + width - 93, y + 21, label, color);
    }

    fn browser_start(&mut self, x: usize, y: usize, width: usize, height: usize) {
        let parsed = Url::parse("nova://start").is_ok();
        let tokens = HtmlTokenizer::new("<main><h1>Nova Browser</h1><p>Start</p></main>").count();
        self.round_rect_blend(x + 7, y + 10, width, height, 17, [0, 0, 8], 110);
        self.round_rect_blend(x, y, width, height, 14, [9, 25, 50], 232);
        self.stroke_rect(x, y, width, height, 14, BORDER_SOFT);
        self.fill_circle(x + 50, y + 48, 20, CYAN);
        self.fill_circle(x + 50, y + 48, 13, [20, 74, 125]);
        self.fill_circle(x + 56, y + 42, 6, MINT);
        self.heading(x + 86, y + 30, "Інтернет у просторі Nova", TEXT);
        self.text(
            x + 86,
            y + 62,
            "Приватний перегляд і локальний контроль даних.",
            MUTED,
        );
        self.round_rect_blend(x + 38, y + 112, width - 76, 48, 14, GLASS_3, 242);
        self.stroke_rect(x + 38, y + 112, width - 76, 48, 14, BORDER_SOFT);
        self.fill_circle(x + 61, y + 136, 7, BORDER);
        self.fill_circle(x + 61, y + 136, 3, GLASS_3);
        self.text(x + 78, y + 128, "Знайти в мережі або ввести адресу", MUTED);
        let card_width = (width - 96) / 3;
        self.browser_card(
            x + 38,
            y + 190,
            card_width,
            "Довідка Nova",
            "Посібник офлайн",
            CYAN,
        );
        self.browser_card(
            x + 48 + card_width,
            y + 190,
            card_width,
            "Проєкти",
            "Відкрити робочі файли",
            VIOLET,
        );
        self.browser_card(
            x + 58 + card_width * 2,
            y + 190,
            card_width,
            "Приватність",
            "Захист без стеження",
            MINT,
        );
        self.fill_rect(
            x + 40,
            y + height - 38,
            8,
            8,
            if parsed && tokens > 0 { MINT } else { RED },
        );
        self.text(x + 58, y + height - 43, "Рушій HTML активний", TEXT);
        self.text(
            x + 220,
            y + height - 43,
            "Драйвер мережі/TLS у розробці",
            YELLOW,
        );
    }

    fn browser_card(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        title: &str,
        detail: &str,
        color: [u8; 3],
    ) {
        self.round_rect_blend(
            x + 3,
            y + 5,
            width.saturating_sub(10),
            112,
            15,
            [0, 0, 8],
            100,
        );
        self.round_rect_blend(x, y, width.saturating_sub(10), 112, 14, GLASS_2, 238);
        self.stroke_rect(x, y, width.saturating_sub(10), 112, 14, BORDER_SOFT);
        self.fill_rect(x + 15, y + 12, 3, 88, color);
        self.fill_circle(x + 28, y + 28, 11, color);
        self.text(x + 48, y + 18, title, TEXT);
        self.text(x + 18, y + 58, detail, MUTED);
    }

    fn quick_settings(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.round_rect_blend(x + 9, y + 12, width, height, 19, [0, 0, 8], 100);
        self.round_rect_blend(x + 4, y + 6, width, height, 18, [0, 0, 8], 140);
        self.round_rect_blend(x, y, width, height, 16, GLASS, 232);
        self.stroke_rect(x, y, width, height, 16, BORDER_SOFT);
        let gap = 9;
        if width < 500 {
            let tile = (width.saturating_sub(37 + gap)) / 2;
            let left = x + 14;
            self.setting_tile(left, y + 15, tile, "Wi-Fi", "Увімкнено", CYAN);
            self.setting_tile(left + tile + gap, y + 15, tile, "Звук", "80%", BLUE);
            self.setting_tile(left, y + 98, tile, "Екран", "70%", VIOLET);
            self.setting_tile(left + tile + gap, y + 98, tile, "Живлення", "Добре", MINT);
        } else {
            let tile = (width.saturating_sub(40 + gap * 3)) / 4;
            let tile_y = y + 16;
            self.setting_tile(x + 14, tile_y, tile, "Wi-Fi", "Увімкнено", CYAN);
            self.setting_tile(x + 14 + tile + gap, tile_y, tile, "Звук", "80%", BLUE);
            self.setting_tile(
                x + 14 + (tile + gap) * 2,
                tile_y,
                tile,
                "Екран",
                "70%",
                VIOLET,
            );
            self.setting_tile(
                x + 14 + (tile + gap) * 3,
                tile_y,
                tile,
                "Живлення",
                "Добре",
                MINT,
            );
        }
        if height > 205 && width >= 500 {
            self.text(x + 20, y + 108, "Гучність", MUTED);
            self.round_rect(x + 94, y + 112, width - 136, 5, 2, [43, 62, 92]);
            self.round_rect(x + 94, y + 112, (width - 136) * 4 / 5, 5, 2, BLUE);
        }
        if height > 300 && width >= 500 {
            self.heading(x + 20, y + 151, "Стан системи", TEXT);
            let card_width = (width - 58) / 3;
            self.setting_tile(x + 20, y + 184, card_width, "Безпека", "Захищено", MINT);
            self.setting_tile(
                x + 29 + card_width,
                y + 184,
                card_width,
                "Оновлення",
                "Актуально",
                CYAN,
            );
            self.setting_tile(
                x + 38 + card_width * 2,
                y + 184,
                card_width,
                "Дані",
                "Локально",
                VIOLET,
            );
        }
        if height > 205 {
            self.fill_circle(x + 24, y + height - 25, 4, MINT);
            self.text(x + 36, y + height - 33, "Система готова", TEXT);
            self.text(x + width - 104, y + height - 33, "Усі параметри", MUTED);
        }
    }

    fn taskbar(&mut self, active: App) {
        let y = self.taskbar_y();
        let x = self.info.width / 26;
        let width = self.info.width - x * 2;
        self.round_rect_blend(x + 10, y + 11, width, 56, 20, [0, 0, 8], 90);
        self.round_rect_blend(x + 5, y + 6, width, 56, 19, [0, 0, 8], 145);
        self.round_rect_blend(x, y, width, 56, 17, [11, 24, 49], 235);
        self.stroke_rect(x, y, width, 56, 17, BORDER_SOFT);
        self.fill_rect(x + 20, y, width.saturating_sub(40), 1, [71, 107, 157]);
        self.fill_circle(x + 32, y + 28, 17, [111, 73, 226]);
        self.fill_circle(x + 32, y + 28, 11, [22, 35, 72]);
        self.fill_circle(x + 32, y + 28, 5, CYAN);
        self.text(x + 58, y + 20, "Nova", TEXT);
        if self.info.width > 1000 {
            self.round_rect_blend(x + 145, y + 10, 190, 36, 18, GLASS_2, 242);
            self.stroke_rect(x + 145, y + 10, 190, 36, 18, BORDER_SOFT);
            self.fill_circle(x + 166, y + 27, 6, BORDER);
            self.fill_circle(x + 166, y + 27, 3, GLASS_2);
            self.text(x + 180, y + 20, "Пошук у Nova", MUTED);
        }
        let center = self.info.width / 2;
        self.task_app(
            center - 245,
            y + 8,
            105,
            "Файли",
            active == App::Files,
            YELLOW,
        );
        self.task_app(
            center - 130,
            y + 8,
            110,
            "Браузер",
            active == App::Browser,
            CYAN,
        );
        self.task_app(
            center - 10,
            y + 8,
            120,
            "Термінал",
            active == App::Terminal,
            BLUE,
        );
        self.task_app(
            center + 120,
            y + 8,
            120,
            "Nova AI",
            active == App::Ai,
            VIOLET,
        );
        self.fill_circle(self.info.width - x - 231, y + 22, 4, MINT);
        self.text(self.info.width - x - 214, y + 14, "Мережа  |  Звук", TEXT);
        self.text(self.info.width - x - 104, y + 14, "10:42", TEXT);
        self.text(self.info.width - x - 104, y + 32, "14 липня", MUTED);
    }

    fn task_app(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        label: &str,
        active: bool,
        color: [u8; 3],
    ) {
        if active {
            self.round_rect_blend(x, y, width, 40, 11, GLASS_3, 244);
            self.stroke_rect(x, y, width, 40, 11, BORDER_SOFT);
            self.round_rect(x + 16, y + 36, width - 32, 3, 1, color);
        }
        if label == "Файли" {
            self.round_rect(x + 11, y + 13, 26, 18, 4, [241, 166, 35]);
            self.fill_rect(x + 14, y + 9, 11, 7, [255, 205, 82]);
            self.fill_rect(x + 14, y + 15, 20, 2, [255, 220, 118]);
        } else if label == "Термінал" {
            self.round_rect(x + 10, y + 8, 28, 25, 5, [28, 43, 70]);
            self.text(x + 15, y + 12, ">_", color);
        } else if label == "Браузер" {
            self.fill_circle(x + 24, y + 20, 12, color);
            self.fill_circle(x + 24, y + 20, 8, [20, 74, 125]);
            self.fill_circle(x + 27, y + 17, 4, MINT);
        } else {
            self.fill_circle(x + 24, y + 20, 12, color);
            self.fill_circle(x + 24, y + 20, 7, [29, 35, 76]);
            self.fill_circle(x + 24, y + 20, 3, CYAN);
        }
        self.text(x + 43, y + 12, label, if active { TEXT } else { MUTED });
    }

    fn setting_tile(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        label: &str,
        value: &str,
        color: [u8; 3],
    ) {
        self.round_rect_blend(x, y, width, 76, 11, GLASS_2, 244);
        self.stroke_rect(x, y, width, 76, 11, BORDER_SOFT);
        self.fill_circle(x + 16, y + 17, 5, color);
        self.text(x + 12, y + 33, label, TEXT);
        self.text(x + 12, y + 53, value, MUTED);
    }

    fn nav_row(&mut self, x: usize, y: usize, width: usize, label: &str, active: bool) {
        if active {
            self.round_rect(x, y, width, 32, 8, GLASS_3);
            self.fill_rect(x, y + 7, 3, 18, BLUE);
        }
        self.text(x + 16, y + 9, label, if active { TEXT } else { MUTED });
    }

    fn file_tile(&mut self, x: usize, y: usize, label: &str, selected: bool) {
        if selected {
            self.round_rect(x - 7, y - 7, 145, 82, 10, GLASS_3);
            self.stroke_rect(x - 7, y - 7, 145, 82, 10, BLUE);
        }
        let name = label.rsplit('/').next().unwrap_or(label);
        if name.contains('.') {
            self.round_rect_blend(x + 43, y + 4, 42, 48, 7, BLUE, 238);
            self.fill_rect(x + 53, y + 16, 22, 2, [222, 235, 255]);
            self.fill_rect(x + 53, y + 24, 18, 2, [222, 235, 255]);
            self.fill_rect(x + 53, y + 32, 21, 2, [222, 235, 255]);
            self.fill_rect(x + 53, y + 40, 14, 2, [222, 235, 255]);
        } else {
            self.round_rect(x + 33, y + 14, 66, 40, 7, [241, 166, 35]);
            self.fill_rect(x + 38, y + 7, 29, 13, [255, 203, 78]);
            self.fill_rect(x + 38, y + 17, 56, 3, [255, 220, 115]);
        }
        let short = utf8_prefix(name, 15);
        self.text(x + 8, y + 62, short, TEXT);
    }

    fn window_controls(&mut self, x: usize, y: usize) {
        self.fill_rect(x, y + 7, 10, 1, MUTED);
        self.stroke_box(x + 35, y + 2, 10, 10, MUTED);
        self.fill_rect(x + 73, y + 2, 12, 12, RED);
    }

    fn heading(&mut self, mut x: usize, y: usize, text: &str, color: [u8; 3]) {
        for ch in text.chars() {
            x += self.inter_glyph(x, y, ch, color, true);
        }
    }

    fn text(&mut self, mut x: usize, y: usize, text: &str, color: [u8; 3]) {
        for ch in text.chars() {
            x += self.inter_glyph(x, y, ch, color, false);
        }
    }

    fn inter_glyph(
        &mut self,
        x: usize,
        y: usize,
        ch: char,
        color: [u8; 3],
        heading: bool,
    ) -> usize {
        let glyph = if heading {
            inter::heading(ch)
        } else {
            inter::body(ch)
        };
        let top = y as isize + glyph.px as isize - glyph.ymin as isize - glyph.height as isize;
        let left = x as isize + glyph.xmin as isize;
        for dy in 0..glyph.height as usize {
            for dx in 0..glyph.width as usize {
                let px = left + dx as isize;
                let py = top + dy as isize;
                if px >= 0 && py >= 0 {
                    let alpha = glyph.bitmap[dy * glyph.width as usize + dx];
                    if alpha > 0 {
                        self.blend_pixel(px as usize, py as usize, color, alpha);
                    }
                }
            }
        }
        glyph.advance as usize
    }

    fn newline(&mut self) {
        self.x = self.content_left;
        self.y += 20;
        if self.y + 18 >= self.content_bottom {
            self.fill_rect(
                self.content_left,
                self.content_top,
                self.content_right - self.content_left,
                self.content_bottom - self.content_top,
                GLASS,
            );
            self.y = self.content_top;
        }
    }

    fn backspace(&mut self) {
        if self.x >= self.content_left + 9 {
            self.x -= 9;
            self.fill_rect(self.x, self.y, 9, 18, GLASS);
        }
    }

    fn character(&mut self, ch: char) {
        if self.x + 9 >= self.content_right {
            self.newline();
        }
        let advance = self.inter_glyph(self.x, self.y, ch, self.foreground, false);
        self.x += advance;
    }

    fn stroke_box(&mut self, x: usize, y: usize, width: usize, height: usize, color: [u8; 3]) {
        self.fill_rect(x, y, width, 1, color);
        self.fill_rect(x, y + height - 1, width, 1, color);
        self.fill_rect(x, y, 1, height, color);
        self.fill_rect(x + width - 1, y, 1, height, color);
    }

    fn stroke_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        _radius: usize,
        color: [u8; 3],
    ) {
        self.fill_rect(x + 12, y, width.saturating_sub(24), 1, color);
        self.fill_rect(x + 12, y + height - 1, width.saturating_sub(24), 1, color);
        self.fill_rect(x, y + 12, 1, height.saturating_sub(24), color);
        self.fill_rect(x + width - 1, y + 12, 1, height.saturating_sub(24), color);
    }

    fn round_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        radius: usize,
        color: [u8; 3],
    ) {
        if width < radius * 2 || height < radius * 2 {
            self.fill_rect(x, y, width, height, color);
            return;
        }
        self.fill_rect(x + radius, y, width - radius * 2, height, color);
        self.fill_rect(x, y + radius, width, height - radius * 2, color);
        let r2 = radius * radius;
        for dy in 0..radius {
            for dx in 0..radius {
                let ox = radius - dx;
                let oy = radius - dy;
                if ox * ox + oy * oy <= r2 {
                    self.pixel(x + dx, y + dy, color);
                    self.pixel(x + width - 1 - dx, y + dy, color);
                    self.pixel(x + dx, y + height - 1 - dy, color);
                    self.pixel(x + width - 1 - dx, y + height - 1 - dy, color);
                }
            }
        }
    }

    fn round_rect_blend(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        radius: usize,
        color: [u8; 3],
        alpha: u8,
    ) {
        if width < radius * 2 || height < radius * 2 {
            self.blend_rect(x, y, width, height, color, alpha);
            return;
        }
        self.blend_rect(x + radius, y, width - radius * 2, height, color, alpha);
        self.blend_rect(x, y + radius, width, height - radius * 2, color, alpha);
        let r2 = radius * radius;
        for dy in 0..radius {
            for dx in 0..radius {
                let ox = radius - dx;
                let oy = radius - dy;
                if ox * ox + oy * oy <= r2 {
                    self.blend_pixel(x + dx, y + dy, color, alpha);
                    self.blend_pixel(x + width - 1 - dx, y + dy, color, alpha);
                    self.blend_pixel(x + dx, y + height - 1 - dy, color, alpha);
                    self.blend_pixel(x + width - 1 - dx, y + height - 1 - dy, color, alpha);
                }
            }
        }
    }

    fn fill_circle(&mut self, cx: usize, cy: usize, radius: usize, color: [u8; 3]) {
        let r2 = (radius * radius) as isize;
        for dy in -(radius as isize)..=radius as isize {
            for dx in -(radius as isize)..=radius as isize {
                if dx * dx + dy * dy <= r2 {
                    let x = cx as isize + dx;
                    let y = cy as isize + dy;
                    if x >= 0 && y >= 0 {
                        self.pixel(x as usize, y as usize, color);
                    }
                }
            }
        }
    }

    fn blend_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: [u8; 3],
        alpha: u8,
    ) {
        for py in y..y.saturating_add(height).min(self.info.height) {
            for px in x..x.saturating_add(width).min(self.info.width) {
                self.blend_pixel(px, py, color, alpha);
            }
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: [u8; 3]) {
        for py in y..y.saturating_add(height).min(self.info.height) {
            for px in x..x.saturating_add(width).min(self.info.width) {
                self.pixel(px, py, color);
            }
        }
    }

    fn pixel(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let at = (y * self.info.stride + x) * self.info.bytes_per_pixel;
        let color = match self.info.pixel_format {
            PixelFormat::Rgb => rgb,
            PixelFormat::Bgr => [rgb[2], rgb[1], rgb[0]],
            _ => [rgb[1], rgb[1], rgb[1]],
        };
        unsafe {
            for (offset, value) in color.iter().enumerate().take(self.info.bytes_per_pixel) {
                ptr::write_volatile(self.buffer.add(at + offset), *value);
            }
        }
    }

    fn blend_pixel(&mut self, x: usize, y: usize, rgb: [u8; 3], alpha: u8) {
        if x >= self.info.width || y >= self.info.height || self.info.bytes_per_pixel < 3 {
            return;
        }
        let at = (y * self.info.stride + x) * self.info.bytes_per_pixel;
        let stored = unsafe {
            [
                ptr::read_volatile(self.buffer.add(at)),
                ptr::read_volatile(self.buffer.add(at + 1)),
                ptr::read_volatile(self.buffer.add(at + 2)),
            ]
        };
        let background = match self.info.pixel_format {
            PixelFormat::Rgb => stored,
            PixelFormat::Bgr => [stored[2], stored[1], stored[0]],
            _ => [stored[0], stored[0], stored[0]],
        };
        let a = alpha as u16;
        let inv = 255 - a;
        let mixed = [
            ((rgb[0] as u16 * a + background[0] as u16 * inv) / 255) as u8,
            ((rgb[1] as u16 * a + background[1] as u16 * inv) / 255) as u8,
            ((rgb[2] as u16 * a + background[2] as u16 * inv) / 255) as u8,
        ];
        self.pixel(x, y, mixed);
    }

    fn pointer(&mut self, x: usize, y: usize) {
        self.restore_pointer();
        self.cursor_x = x.min(self.info.width.saturating_sub(18));
        self.cursor_y = y.min(self.info.height.saturating_sub(24));
        self.toggle_pointer();
        self.cursor_visible = true;
    }

    fn toggle_pointer(&mut self) {
        for dy in 0..20 {
            let edge = (dy / 2 + 1).min(11);
            for dx in 0..edge {
                self.xor_pixel(self.cursor_x + dx, self.cursor_y + dy);
            }
        }
        for dy in 15..22 {
            for dx in 7..11 {
                self.xor_pixel(self.cursor_x + dx, self.cursor_y + dy);
            }
        }
    }

    fn restore_pointer(&mut self) {
        if self.cursor_visible {
            self.toggle_pointer();
            self.cursor_visible = false;
        }
    }

    fn xor_pixel(&mut self, x: usize, y: usize) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let at = (y * self.info.stride + x) * self.info.bytes_per_pixel;
        unsafe {
            for byte in 0..self.info.bytes_per_pixel {
                let address = self.buffer.add(at + byte);
                ptr::write_volatile(address, ptr::read_volatile(address) ^ 0xff);
            }
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for ch in text.chars() {
            match ch {
                '\n' => self.newline(),
                '\x08' => self.backspace(),
                ch => self.character(ch),
            }
        }
        Ok(())
    }
}

fn utf8_prefix(text: &str, max_chars: usize) -> &str {
    let end = text
        .char_indices()
        .nth(max_chars)
        .map_or(text.len(), |(at, _)| at);
    &text[..end]
}

#[macro_export]
macro_rules! print { ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*))); }
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
