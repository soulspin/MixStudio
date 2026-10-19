use super::*;

impl App {
    fn lib_import_settings(&mut self, ui: &mut egui::Ui) {
        let dim = |s: &str| egui::RichText::new(s.to_string()).color(DIM).size(11.0);
        ui.separator();
        ui.label(egui::RichText::new("Other DJ software").strong().size(12.0));
        let mut sel: Option<Option<(usize, Vec<usize>)>> = None; let mut rm: Option<usize> = None;
        let cur = self.lib_node.clone();
        for (li, f) in self.lib.cfg.imports.clone().iter().enumerate() {
            match self.lib.ext.get(li) {
                Some(Ok(l)) => {
                    let missing = l.tracks.iter().filter(|t| !t.exists).count();
                    egui::CollapsingHeader::new(format!("{} ({} tracks)", l.source.name(), l.tracks.len())).id_salt(("imp", li)).default_open(true).show(ui, |ui| {
                        ui.add(egui::Label::new(dim(f)).truncate()).on_hover_text(f);
                        if missing > 0 { ui.label(egui::RichText::new(format!("⚠ {missing} files not found: check path remapping")).color(C::from_rgb(230, 170, 60)).size(11.0)); }
                        let c = cur.as_ref().map(|n| (n.0, n.1.as_slice()));
                        if ui.selectable_label(c == Some((li, &[][..])), format!("♫ All tracks ({})", l.tracks.len())).clicked() { sel = Some(Some((li, vec![]))); }
                        let mut path = vec![]; for (ci, ch) in l.root.children.iter().enumerate() { path.push(ci); node_ui(ui, ch, li, &mut path, c, &mut sel); path.pop(); }
                        if ui.small_button("✖ Remove import").on_hover_text("Forget this import (the other app's files are never touched)").clicked() { rm = Some(li); }
                    });
                }
                Some(Err(e)) => { ui.label(egui::RichText::new(format!("⚠ {f}: {e}")).color(C::from_rgb(230, 100, 80)).size(11.0)); if ui.small_button("✖ Remove").clicked() { rm = Some(li); } }
                None => { ui.horizontal(|ui| { ui.spinner(); ui.label(dim(&format!("Reading {f}…"))); }); }
            }
        }
        if let Some(s) = sel { self.library_ui.folder=None; self.lib_node = s; self.lib_sel.clear(); self.lib_anchor = None; }
        if let Some(i) = rm { self.lib.remove_import(i); self.lib_node = None; self.lib_sel.clear(); }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("🔍 Find libraries").on_hover_text("Look in the usual places: home folder, Documents, Music and mounted drives (incl. Windows / macOS user folders)").clicked() { self.find_libs(); }
            if ui.button("Browse…").clicked() { self.pick_import(); } });
        for p in self.lib_found.clone() { let s = p.display().to_string(); let have = self.lib.cfg.imports.contains(&s);
            ui.horizontal(|ui| { let k = crate::import::sniff(&p).map(|k| k.name()).unwrap_or("?");
                if ui.add_enabled(!have, egui::Button::new(if have { "✔" } else { "Import" }).small()).clicked() { self.import_file(p.clone()); }
                ui.add(egui::Label::new(dim(&format!("{k}: {s}"))).truncate()).on_hover_text(&s); }); }
        ui.horizontal(|ui| { ui.add(egui::TextEdit::singleline(&mut self.lib_imp).hint_text("…or paste a library file path").desired_width(150.0));
            if ui.add_enabled(!self.lib_imp.trim().is_empty(), egui::Button::new("Import")).clicked() { let p = PathBuf::from(self.lib_imp.trim()); self.lib_imp.clear(); self.import_file(p); } });
        ui.label(dim("rekordbox.xml (File ▸ Export Collection in xml format), Traktor collection.nml, _Serato_/database V2, Engine Library/Database2/m.db, iTunes / Music Library.xml. Read-only."));
        ui.separator();
        egui::CollapsingHeader::new("Path remapping").id_salt("remaps").show(ui, |ui| {
            if !self.lib_remaps_init { self.lib_remaps = self.lib.cfg.remaps.clone(); self.lib_remaps_init = true; }
            ui.label(dim("Rewrite path prefixes of imported tracks, e.g. C:\\Users\\Mike\\Music → /mnt/windows/Users/Mike/Music"));
            let mut del = None;
            for (i, r) in self.lib_remaps.iter_mut().enumerate() { ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut r.from).hint_text("from").desired_width(95.0)); ui.label("→");
                ui.add(egui::TextEdit::singleline(&mut r.to).hint_text("to").desired_width(95.0)); if ui.small_button("✖").clicked() { del = Some(i); } }); }
            if let Some(i) = del { self.lib_remaps.remove(i); }
            ui.horizontal(|ui| { if ui.button("＋ Add rule").clicked() { self.lib_remaps.push(Default::default()); }
                let changed = self.lib_remaps != self.lib.cfg.remaps;
                if ui.add_enabled(changed, egui::Button::new("Apply")).clicked() { let v: Vec<_> = self.lib_remaps.iter().filter(|r| !r.from.trim().is_empty()).cloned().collect(); self.lib_remaps = v.clone(); self.lib.set_remaps(v); self.lib_cur_key = None; } });
        });
    }
}

fn node_ui(ui: &mut egui::Ui, n: &crate::import::Node, li: usize, path: &mut Vec<usize>, cur: Option<(usize, &[usize])>, sel: &mut Option<Option<(usize, Vec<usize>)>>) {
    let me = cur == Some((li, path.as_slice()));
    if n.children.is_empty() { if ui.selectable_label(me, format!("♫ {} ({})", n.name, n.tracks.len())).clicked() { *sel = Some(Some((li, path.clone()))); } return; }
    egui::CollapsingHeader::new(format!("🗀 {}", n.name)).id_salt(("node", li, path.clone())).show(ui, |ui| {
        if ui.selectable_label(me, format!("♫ All in {} ({})", n.name, n.all_tracks().len())).clicked() { *sel = Some(Some((li, path.clone()))); }
        for (ci, c) in n.children.iter().enumerate() { path.push(ci); node_ui(ui, c, li, path, cur, sel); path.pop(); }
    });
}

#[derive(Default)]
pub(super) struct State {
    pub folder:Option<String>,pub details:bool,pub geometry:Option<library_drop::Geometry>,
    tree:Vec<crate::library::browse::FolderNode>,tree_key:Option<(u64,Vec<String>)>,
}
fn folder_node(ui:&mut egui::Ui,node:&crate::library::browse::FolderNode,current:Option<&str>,selected:&mut Option<String>,remove:&mut Option<String>,root:bool) {
    let row=|ui:&mut egui::Ui,selected:&mut Option<String>,remove:&mut Option<String>| {
        let r=ui.selectable_label(current==Some(node.path.as_str()),format!("{} ({})",node.name,node.count));
        if r.clicked(){*selected=Some(node.path.clone());}
        let r=r.on_hover_text(&node.path);
        if root {r.context_menu(|ui|{if ui.button("Remove from library").clicked(){*remove=Some(node.path.clone());ui.close_menu();}});}
    };
    if node.children.is_empty(){ui.horizontal(|ui|{ui.add_space(18.0);row(ui,selected,remove);});}
    else {egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(),ui.id().with(("music_folder",&node.path)),root)
        .show_header(ui,|ui|row(ui,selected,remove)).body(|ui|{for child in &node.children {folder_node(ui,child,current,selected,remove,false);}});}
}
impl App {
    fn library_entries(&self)->&[crate::library::Entry] {if self.lib_node.is_some(){&self.lib_cur}else{&self.lib.index.entries}}
    pub(super) fn library_selected_paths(&self)->Vec<String> {
        self.lib_view.iter().filter_map(|&i|self.library_entries().get(i)).filter(|e|self.lib_sel.contains(&e.path)).map(|e|e.path.clone()).collect()
    }
    fn choose_library_folder(&mut self,folder:Option<String>) {
        self.library_ui.folder=folder;self.lib_node=None;self.lib_sel.clear();self.lib_anchor=None;self.lib_view_key=None;
    }
    fn library_sidebar(&mut self,ui:&mut egui::Ui) {
        ui.heading("Music folders");
        if ui.button("+ Add folder…").clicked(){self.pick_folder();}
        if ui.selectable_label(self.lib_node.is_none()&&self.library_ui.folder.is_none(),format!("All music ({})",self.lib.index.entries.len())).clicked(){self.choose_library_folder(None);}
        let key=(self.lib.path_gen,self.lib.cfg.folders.clone());
        if self.library_ui.tree_key.as_ref()!=Some(&key){self.library_ui.tree=crate::library::browse::folder_tree(&self.lib.cfg.folders,&self.lib.index.entries);self.library_ui.tree_key=Some(key);}
        let mut selected=None;let mut remove=None;
        for node in &self.library_ui.tree {folder_node(ui,node,self.library_ui.folder.as_deref(),&mut selected,&mut remove,true);}
        if let Some(folder)=selected{self.choose_library_folder(Some(folder));}
        if let Some(folder)=remove{self.lib.remove_folder(&folder);self.choose_library_folder(None);}
        ui.separator();
        egui::CollapsingHeader::new("DJ software imports").id_salt("library_import_settings").show(ui,|ui|self.lib_import_settings(ui));
        egui::CollapsingHeader::new("Manage sources").id_salt("library_sources").show(ui,|ui|{
            if ui.add_enabled(!self.lib.cfg.folders.is_empty(),egui::Button::new("Rescan folders")).clicked(){self.lib.rescan();}
            ui.add(egui::TextEdit::singleline(&mut self.lib_path).hint_text("Paste a music folder path").desired_width(ui.available_width()));
            if ui.add_enabled(!self.lib_path.trim().is_empty(),egui::Button::new("Add path")).clicked(){let path=PathBuf::from(self.lib_path.trim());self.lib_path.clear();self.add_lib_folders(vec![path]);}
            for root in &self.lib.cfg.folders {let status=match self.lib.states.get(root){Some(crate::library::FolderState::Available(_))=>"Available",Some(crate::library::FolderState::Partial(_))=>"Partial scan",Some(crate::library::FolderState::Missing)=>"Drive unavailable",Some(crate::library::FolderState::NotResponding)=>"Drive not responding",None=>"Waiting for scan"};
                ui.add(egui::Label::new(egui::RichText::new(format!("{status}: {root}")).size(11.0).color(DIM)).wrap());}
        });
    }
    pub(super) fn refresh_library_view(&mut self) {
        if let Some(node)=self.lib_node.clone(){if self.lib_cur_key.as_ref()!=Some(&(self.lib.gen,node.clone())){
            self.lib_cur=match self.lib.ext.get(node.0).and_then(|r|r.as_ref().ok()) {Some(l)=>{
                let ids=if node.1.is_empty(){(0..l.tracks.len()).collect()}else{l.root.at(&node.1).map(|n|n.all_tracks()).unwrap_or_default()};let mut seen=std::collections::HashSet::new();
                ids.into_iter().filter(|&i|seen.insert(i)).filter_map(|i|l.tracks.get(i)).map(|t|{let ours=self.lib.entry(&t.path);crate::library::Entry{path:t.path.clone(),folder:crate::library::EXT_FOLDER.into(),title:t.title.clone(),artist:t.artist.clone(),album:t.album.clone(),genre:t.genre.clone(),duration:if t.duration>0.0{t.duration}else{ours.map(|e|e.duration).unwrap_or(0.0)},bpm:ours.and_then(|e|e.bpm),key:ours.and_then(|e|e.key.clone()),error:if t.exists{None}else{Some("Source file unavailable; check path remapping".into())},..Default::default()}}).collect()
            },None=>{self.lib_node=None;vec![]}};self.lib_cur_key=Some((self.lib.gen,node));self.lib_view_key=None;
        }}
        let reference=self.lib_ref();let key_ref=if self.lib_key{reference.as_ref().map(|r|r.1.clone())}else{None};let bpm_ref=if self.lib_bpm{reference.as_ref().map(|r|(r.2,self.lib_pct))}else{None};
        let gen={use std::hash::{Hash,Hasher};let mut h=std::collections::hash_map::DefaultHasher::new();self.lib_node.hash(&mut h);self.library_ui.folder.hash(&mut h);self.lib.gen^h.finish()};
        let key=(gen,self.lib_q.clone(),self.lib_sort,self.lib_asc,key_ref.clone(),bpm_ref.map(|(b,p)|(b.to_bits(),p.to_bits())));
        if self.lib_view_key.as_ref()!=Some(&key){
            self.lib_view=crate::library::browse::folder_view(self.library_entries(),&crate::library::Filter{query:&self.lib_q,key_ref:key_ref.as_deref(),bpm_ref},self.lib_sort,self.lib_asc,if self.lib_node.is_none(){self.library_ui.folder.as_deref()}else{None});
            let visible:std::collections::HashSet<_>=self.lib_view.iter().map(|&i|self.library_entries()[i].path.clone()).collect();self.lib_sel.retain(|p|visible.contains(p));if self.lib_anchor.as_ref().is_some_and(|p|!visible.contains(p)){self.lib_anchor=None;}self.lib_view_key=Some(key);
        }
    }
    pub(super) fn library_tab(&mut self,ui:&mut egui::Ui) {
        egui::SidePanel::left("library_navigation_v2").resizable(true).default_width(200.0).width_range(160.0..=340.0).show_inside(ui,|ui|{egui::ScrollArea::vertical().id_salt("library_navigation_scroll").show(ui,|ui|self.library_sidebar(ui));});
        use crate::library::SortCol;
        let mut append=false;
        ui.horizontal(|ui|{
            let width=(ui.available_width()-205.0).max(120.0);
            ui.add(egui::TextEdit::singleline(&mut self.lib_q).hint_text("Search filename, title, artist…").desired_width(width));
            if ui.small_button("Clear").clicked(){self.lib_q.clear();}
            let n=self.lib_sel.len();if ui.add_enabled(n>0&&!self.arrangement_locked(),egui::Button::new(egui::RichText::new(format!("Add last in mix ({n})")).color(if n>0{C::BLACK}else{DIM})).fill(if n>0{ACCENT}else{PANEL})).clicked(){append=true;}
        });
        ui.horizontal_wrapped(|ui|{
            ui.checkbox(&mut self.lib_key,"Compatible key");ui.checkbox(&mut self.lib_bpm,"BPM ±");ui.add(egui::DragValue::new(&mut self.lib_pct).range(1.0..=30.0).speed(0.2).suffix("%"));
            egui::ComboBox::from_id_salt("library_sort").width(100.0).selected_text(sort_name(self.lib_sort)).show_ui(ui,|ui|{for col in [SortCol::Filename,SortCol::Title,SortCol::Artist,SortCol::Bpm,SortCol::Key,SortCol::Duration,SortCol::Folder,SortCol::Genre]{ui.selectable_value(&mut self.lib_sort,col,sort_name(col));}});
            if ui.small_button(if self.lib_asc{"Ascending"}else{"Descending"}).clicked(){self.lib_asc=!self.lib_asc;}
            ui.checkbox(&mut self.library_ui.details,"Details");
        });
        if self.lib_key||self.lib_bpm {ui.label(egui::RichText::new(self.lib_ref().map(|(t,k,b)|format!("Match: {t} · {k} · {b:.1} BPM")).unwrap_or_else(||"Add/select a mix track as filter reference".into())).size(11.0).color(DIM));}
        self.refresh_library_view();
        ui.horizontal(|ui|{
            ui.label(egui::RichText::new(format!("{} files · {} selected",self.lib_view.len(),self.lib_sel.len())).color(DIM).size(11.0));
            if self.lib.scanning{ui.spinner();ui.label("Scanning folders…");}else if self.lib.analysing.1>0{ui.label(format!("Analysing BPM/key {}/{}",self.lib.analysing.0,self.lib.analysing.1));}else{ui.label(egui::RichText::new("Drag a row onto the timeline · Ctrl/Shift selects multiple").size(11.0).color(DIM));}
        });
        if self.lib_view.is_empty(){ui.label(if self.lib.cfg.folders.is_empty()&&self.lib_node.is_none(){"Add a music folder to start your library."}else{"No files match. Clear search or filters, or choose another folder."});}
        let mut columns=vec![(SortCol::Filename,"Filename / artist and title"),(SortCol::Bpm,"BPM"),(SortCol::Key,"Key"),(SortCol::Duration,"Length")];
        if self.library_ui.details {columns.extend([(SortCol::Folder,"Folder"),(SortCol::Genre,"Genre")]);}
        let available=ui.available_height();let mods=ui.input(|i|i.modifiers);
        let mut table=egui_extras::TableBuilder::new(ui).id_salt("library_files_v2").striped(true).resizable(true).sense(Sense::click_and_drag()).max_scroll_height(available).min_scrolled_height(0.0)
            .column(egui_extras::Column::remainder().at_least(180.0)).column(egui_extras::Column::initial(62.0).at_least(45.0)).column(egui_extras::Column::initial(48.0).at_least(35.0)).column(egui_extras::Column::initial(60.0).at_least(45.0));
        if self.library_ui.details{table=table.column(egui_extras::Column::initial(120.0)).column(egui_extras::Column::initial(80.0));}
        let mut sorted=None;let mut click=None;let mut double=None;let mut drag=None;
        table.header(22.0,|mut header|{for (col,name) in &columns {header.col(|ui|{if ui.selectable_label(self.lib_sort==*col,*name).clicked(){sorted=Some(*col);}});}})
            .body(|body|{body.rows(40.0,self.lib_view.len(),|mut row|{
                let k=row.index();let e=&self.library_entries()[self.lib_view[k]];let selected=self.lib_sel.contains(&e.path);row.set_selected(selected);
                let unavailable=e.error.is_some() || (e.folder!=crate::library::EXT_FOLDER && matches!(self.lib.states.get(&e.folder),Some(crate::library::FolderState::Missing|crate::library::FolderState::NotResponding)));
                let color=if unavailable{DIM}else{TXT};
                row.col(|ui|{ui.vertical(|ui|{
                    ui.add(egui::Label::new(egui::RichText::new(e.filename()).strong().size(13.0).color(color)).truncate());
                    let metadata=if e.artist.is_empty(){e.title.clone()}else if e.title.is_empty(){e.artist.clone()}else{format!("{} · {}",e.artist,e.title)};
                    ui.add(egui::Label::new(egui::RichText::new(metadata).size(11.0).color(DIM)).truncate());
                });});
                for text in [e.bpm.map(|b|format!("{b:.1}")).unwrap_or_else(||"—".into()),e.key.clone().unwrap_or_else(||"—".into()),fmt_t(e.duration)]{row.col(|ui|{ui.add(egui::Label::new(egui::RichText::new(text).color(color).size(12.0)).truncate());});}
                if self.library_ui.details{for text in [PathBuf::from(&e.path).parent().map(|p|p.display().to_string()).unwrap_or_default(),e.genre.clone()]{row.col(|ui|{ui.add(egui::Label::new(egui::RichText::new(text).size(11.0).color(DIM)).truncate());});}}
                let response=row.response().on_hover_ui(|ui|{ui.set_max_width(480.0);ui.strong(e.filename());ui.label(format!("{} · {}",e.artist,e.title));ui.label(&e.path);
                    if let Some((source,track))=self.lib.ext_info(&e.path){ui.label(format!("{} tags: {} BPM · {}",source.name(),track.bpm.map(|b|format!("{b:.2}")).unwrap_or_else(||"unknown".into()),track.key.as_deref().unwrap_or("unknown")));}
                    if let Some(error)=&e.error{ui.colored_label(C::LIGHT_RED,error);}else if unavailable{ui.colored_label(C::YELLOW,"Source drive is unavailable");}
                });
                if response.double_clicked(){double=Some(e.path.clone());}else if response.clicked(){click=Some((k,mods));}
                if response.drag_started()&&!self.arrangement_locked(){let paths=if selected{self.library_selected_paths()}else{vec![e.path.clone()]};response.dnd_set_drag_payload(LibDrag(paths.clone()));drag=Some(paths);}
            });});
        if let Some(col)=sorted{if self.lib_sort==col{self.lib_asc=!self.lib_asc;}else{self.lib_sort=col;self.lib_asc=true;}}
        if let Some(paths)=drag{self.lib_sel=paths.into_iter().collect();}
        if let Some((k,mods))=click{self.select_library_row(k,mods);}
        if let Some(path)=double{self.lib_add(vec![path]);}else if append{self.lib_add(self.library_selected_paths());}
    }
    pub(super) fn select_library_row(&mut self,k:usize,mods:egui::Modifiers) {
        let Some(entry)=self.lib_view.get(k).and_then(|&i|self.library_entries().get(i))else{return;};let path=entry.path.clone();
        if mods.shift{let anchor=self.lib_anchor.as_ref().and_then(|p|self.lib_view.iter().position(|&i|self.library_entries()[i].path==*p)).unwrap_or(k);self.lib_anchor=Some(self.library_entries()[self.lib_view[anchor]].path.clone());let paths:Vec<_>=self.lib_view[anchor.min(k)..=anchor.max(k)].iter().map(|&i|self.library_entries()[i].path.clone()).collect();if !mods.command{self.lib_sel.clear();}self.lib_sel.extend(paths);}
        else if mods.command{if !self.lib_sel.remove(&path){self.lib_sel.insert(path.clone());}self.lib_anchor=Some(path.clone());}
        else{self.lib_sel.clear();self.lib_sel.insert(path.clone());self.lib_anchor=Some(path.clone());}
    }
}
fn sort_name(col:crate::library::SortCol)->&'static str {use crate::library::SortCol::*;match col{Filename=>"Filename",Title=>"Title",Artist=>"Artist",Bpm=>"BPM",Key=>"Key",Duration=>"Length",Genre=>"Genre",Folder=>"Folder"}}
