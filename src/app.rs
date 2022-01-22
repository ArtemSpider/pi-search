use std::{sync::mpsc::{Receiver, TryRecvError}, fs::File, io::{Read, Write}};

use eframe::{egui::{self, Ui}, epi};

use crate::search::*;

struct InputInfo {}

impl InputInfo {
    fn new() -> Self {
        Self {
        }
    }
}

struct PreloadInfo {
    preload_size: usize,

    loaded_size_rec: Receiver<usize>,
    loaded_size: usize,

    result_rec: Receiver<()>,
}

impl PreloadInfo {
    fn new(_input_info: &InputInfo, search: &mut Search, preload_size: usize) -> Self {
        let (loaded_size_rec, result_rec) = search.preload(preload_size, 10);
        Self {
            preload_size,
            loaded_size_rec,
            loaded_size: 0usize,
            result_rec,
        }
    }
}

struct SearchInfo {
    processed_size_rec: Receiver<usize>,
    processed_size: usize,

    result_rec: Receiver<Option<usize>>,
}

impl SearchInfo {
    fn new(_input_info: &InputInfo, search: &mut Search, search_for: &str) -> Self {
        let (processed_size_rec, result_rec) = search.search(search_for);
        Self {
            processed_size_rec,
            processed_size: 0usize,
            result_rec,
        }
    }
}

struct FoundInfo {
    index: Option<usize>,
    processed: usize,
}

impl FoundInfo {
    fn new(search_info: &SearchInfo, index: Option<usize>) -> Self {
        Self {
            index,
            processed: search_info.processed_size,
        }
    }
}

enum AppState {
    Input(InputInfo),
    Preload(PreloadInfo),
    Search(SearchInfo),
    Found(FoundInfo),
}

pub struct TemplateApp {
    state: AppState,
    preload_size: String,
    load_size: String,
    search_for: String,
    search: Search,
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            state: AppState::Input(InputInfo::new()),
            preload_size: Default::default(),
            load_size: Default::default(),
            search_for: Default::default(),
            search: Search::new(),
        }
    }
}

impl TemplateApp {
    fn show_files_control(&mut self, ui: &mut Ui) {
        if ui.button("Read all")
             .on_hover_text("Read all digits stored in pi.txt")
             .clicked()
        {
            if self.load_digits().is_err() {
                eprintln!("Error while loading digits");
            }
        }

        ui.horizontal(|ui| {
            if ui.button("Read")
                 .on_hover_text("Read digits stored in pi.txt")
                 .clicked()
            {
                if self.load_size.len() > 0 && self.load_size.chars().all(char::is_numeric) {
                    if self.load_n_digits(self.load_size.parse().unwrap()).is_err() {
                        eprintln!("Error while loading digits");
                    }
                }
            }
            ui.text_edit_singleline(&mut self.load_size);
        });

        if ui.button("Write loaded")
             .on_hover_text("Write all loaded digits to pi.txt")
             .clicked()
        {
            if self.save_digits().is_err() {
                eprintln!("Error while saving digits");
            }
        }
        
        ui.add_space(10f32);
    }

    fn input_state(&mut self, ui: &mut Ui) {
        self.show_files_control(ui);

        if let AppState::Input(info) = &mut self.state {
            ui.label(format!("Digits loaded: {}", self.search.digits_loaded()));

            let mut new_state = None;
            egui::Grid::new("input_grid").max_col_width(120f32).show(ui, |ui| {
                ui.label("Preload: ");
                ui.add_enabled(true, egui::TextEdit::singleline(&mut self.preload_size));
                if ui.button("Preload").clicked() {
                    if self.preload_size.len() > 0 && self.preload_size.chars().all(char::is_numeric) {
                        new_state = Some(AppState::Preload(PreloadInfo::new(&info, &mut self.search, self.preload_size.parse().unwrap())));
                    }
                }
                ui.end_row();

                ui.label("Search for: ");
                ui.add_enabled(true, egui::TextEdit::singleline(&mut self.search_for));
                if ui.button("Search").clicked() {
                    if self.search_for.len() > 0 && self.search_for.chars().all(char::is_numeric) {
                        new_state = Some(AppState::Search(SearchInfo::new(&info, &mut self.search, self.search_for.as_str())));
                    }
                }
                ui.end_row();
            });

            if new_state.is_some() {
                self.state = new_state.unwrap();
            }
        }
    }

    fn preload_state(&mut self, ui: &mut Ui) {
        self.show_files_control(ui);
        
        if let AppState::Preload(info) = &mut self.state {
            ui.label("Preloading...");
            
            loop {
                let loaded_size_res = info.loaded_size_rec.try_recv();
                match loaded_size_res {
                    Ok(loaded_size) => info.loaded_size = loaded_size,
                    Err(_) => { break; },
                }
            }

            ui.label(format!("Loaded {}/{} ({}%)", info.loaded_size, info.preload_size, (info.loaded_size as f32 / info.preload_size as f32 * 100f32) as u32));
        
            let result_res = info.result_rec.try_recv();
            match result_res {
                Ok(_) => {
                    self.search.into_idle();
                    self.state = AppState::Input(InputInfo::new());
                },
                Err(err) => {
                    match err {
                        TryRecvError::Empty => {},
                        TryRecvError::Disconnected => { eprintln!("Preload thread is dead"); },
                    }
                },
            }
        }
    }

    fn search_state(&mut self, ui: &mut Ui) {
        self.show_files_control(ui);
        
        if let AppState::Search(info) = &mut self.state {
            ui.horizontal(|ui| {
                ui.label("Search for: ");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut self.search_for));
            });

            loop {
                let processed_res = info.processed_size_rec.try_recv();
                match processed_res {
                    Ok(pro) => info.processed_size = pro,
                    Err(_) => { break; },
                }
            }
            
            ui.label(format!("Processed: {}", info.processed_size));

            let result_res = info.result_rec.try_recv();
            match result_res {
                Ok(index) => {
                    loop {
                        let processed_res = info.processed_size_rec.try_recv();
                        match processed_res {
                            Ok(pro) => info.processed_size = pro,
                            Err(_) => { break; },
                        }
                    }

                    self.search.into_idle();
                    self.state = AppState::Found(FoundInfo::new(&info, index));
                },
                Err(err) => {
                    match err {
                        TryRecvError::Empty => {},
                        TryRecvError::Disconnected => { panic!("Search thread is dead"); },
                    }
                },
            }
        }
    }

    fn found_state(&mut self, ui: &mut Ui) {
        if let AppState::Found(info) = &mut self.state {
            ui.horizontal(|ui| {
                ui.label("Search for: ");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut self.search_for));
            });

            ui.label(format!("Processed: {}", info.processed));
            ui.label(format!("Index: {:?}", info.index));

            if ui.button("Back").clicked() {
                self.search_for.clear();
                self.state = AppState::Input(InputInfo::new());
            }
        }
    }

    fn load_digits(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut pi_file = File::open("pi.txt")?;
        let digits = self.search.get_digits();

        let file_size = pi_file.metadata()?.len() as usize;
        let digits_size = digits.lock().unwrap().len();

        if file_size > digits_size {
            let mut file_digits = String::default();
            pi_file.read_to_string(&mut file_digits)?;

            digits.lock().unwrap().clear();
            digits.lock().unwrap().push_str(file_digits.as_str());
        }

        Ok(())
    }

    fn load_n_digits(&mut self, count: usize) -> Result<(), Box<dyn std::error::Error>> {
        let mut pi_file = File::open("pi.txt")?;
        let digits = self.search.get_digits();

        let load_size = count.min(pi_file.metadata()?.len() as usize);
        let digits_size = digits.lock().unwrap().len();

        if load_size > digits_size {
            let mut buf = vec![0u8; load_size];
            pi_file.read_exact(&mut buf)?;
            let file_digits = String::from_utf8(buf)?;

            digits.lock().unwrap().clear();
            digits.lock().unwrap().push_str(file_digits.as_str());
        }

        Ok(())
    }

    fn save_digits(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut pi_file = File::create("pi.txt")?;
        let digits = self.search.get_digits();

        let file_size = pi_file.metadata()?.len() as usize;
        let digits_size = digits.lock().unwrap().len();

        if file_size < digits_size {
            pi_file.write_all(&digits.lock().unwrap().as_bytes()[file_size..])?;
        }

        Ok(())
    }
}

impl epi::App for TemplateApp {
    fn name(&self) -> &str {
        "PI Search"
    }

    fn setup(
        &mut self,
        _ctx: &egui::CtxRef,
        _frame: &epi::Frame,
        _storage: Option<&dyn epi::Storage>,
    ) {}

    fn update(&mut self, ctx: &egui::CtxRef, _: &epi::Frame) {
        ctx.request_repaint();

        egui::CentralPanel::default().show(ctx, |ui| {
            match &self.state {
                AppState::Input(_) => self.input_state(ui),
                AppState::Preload(_) => self.preload_state(ui),
                AppState::Search(_) => self.search_state(ui),
                AppState::Found(_) => self.found_state(ui),
            }
        });
    }
}
