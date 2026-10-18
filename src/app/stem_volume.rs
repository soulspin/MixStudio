//! One volume envelope per separated stem, edited directly on its timeline row.
use super::*;
pub(super) struct Edit {ti:usize,k:usize,kind:u8,x:f64,value:f32,rect:Rect}
#[derive(Clone,Copy)]
pub(super) struct Drag {ti:usize,k:usize,index:usize,rect:Rect}
pub(super) fn draw(ui:&egui::Ui,tp:&egui::Painter,ti:usize,t:&Track,wave:Rect,vis:Rect,bx:&impl Fn(f64)->f32,xb:&impl Fn(f32)->f64)->Option<Edit> {
    let mut edit=None;
    for stem in 0..4 {
        let k=5+stem;let h=wave.height()/4.0;
        let r=Rect::from_min_max(pos2(vis.left(),wave.top()+h*stem as f32+12.0),pos2(vis.right(),wave.top()+h*(stem+1) as f32-3.0));
        if r.height()<5.0 {continue;}
        let y=|v:f32|r.bottom()-v*r.height();let points=t.auto.get(k).map(Vec::as_slice).unwrap_or(&[]);
        tp.rect_filled(r,0.0,C::from_black_alpha(70));
        let mut line=vec![pos2(r.left(),y(t.auto_at(k,xb(r.left())).unwrap_or(1.0)))];
        line.extend(points.iter().map(|&(b,v)|pos2(bx(t.start_beat+b),y(v))).filter(|p|p.x>=r.left()&&p.x<=r.right()));
        line.push(pos2(r.right(),y(t.auto_at(k,xb(r.right())).unwrap_or(1.0))));
        tp.add(egui::Shape::line(line,Stroke::new(1.8_f32,STEM[stem])));
        for &(b,v) in points {let p=pos2(bx(t.start_beat+b),y(v));if r.expand(4.0).contains(p){tp.circle_filled(p,3.5,C::WHITE);}}
        let resp=ui.interact(r.expand2(vec2(0.0,3.0)),ui.id().with(("stem_volume",ti,stem)),Sense::click_and_drag());
        if let Some(p)=resp.interact_pointer_pos() {
            if !ui.input(|i|i.modifiers.alt || i.modifiers.command) {
                let near=points.iter().enumerate().filter(|(_,q)|pos2(bx(t.start_beat+q.0),y(q.1)).distance(p)<8.0).min_by(|a,b|((bx(t.start_beat+a.1.0)-p.x).abs()).total_cmp(&((bx(t.start_beat+b.1.0)-p.x).abs()))).map(|(i,_)|i);
                let value=((r.bottom()-p.y)/r.height()).clamp(0.0,1.0);
                let action=if resp.secondary_clicked(){near.map(|i|(2,i as f64))}else if resp.drag_started(){Some(near.map(|i|(1,i as f64)).unwrap_or((0,xb(p.x)-t.start_beat)))}else if resp.clicked()&&near.is_none(){Some((0,xb(p.x)-t.start_beat))}else{None};
                if let Some((kind,x))=action {edit=Some(Edit{ti,k,kind,x,value,rect:r});}
            }
        }
        resp.on_hover_text(format!("{} volume: click to add a point, drag to adjust, right-click a point to remove. 0–100%.",STEM_N[stem]));
    }
    edit
}
impl App {
    pub(super) fn edit_stem_volume(&mut self,edit:Edit) {
        if self.arrangement_locked(){return;}
        self.snapshot();let t=&mut self.mix.tracks[edit.ti];
        match edit.kind {
            2=>{if let Some(p)=t.auto.get_mut(edit.k){p.remove(edit.x as usize);}}
            _=>{let index=if edit.kind==1 {edit.x as usize}else{let x=if self.snapping==Snapping::Off{edit.x}else{(edit.x*4.0).round()/4.0};t.auto_set(edit.k,x,edit.value)};
                self.stem_volume_drag=Some(Drag{ti:edit.ti,k:edit.k,index,rect:edit.rect});}
        }
    }
    pub(super) fn update_stem_volume(&mut self,ui:&egui::Ui,xb:&impl Fn(f32)->f64) {
        let Some(d)=self.stem_volume_drag else{return;};
        if !ui.input(|i|i.pointer.primary_down())||self.arrangement_locked(){self.stem_volume_drag=None;return;}
        let Some(p)=ui.input(|i|i.pointer.hover_pos())else{return;};let Some(t)=self.mix.tracks.get_mut(d.ti)else{self.stem_volume_drag=None;return;};
        let (lo,hi)=(t.vis_start(0.0)-t.start_beat,t.end(0.0)-t.start_beat);
        let mut x=(xb(p.x)-t.start_beat).clamp(lo,hi);if self.snapping!=Snapping::Off{x=(x*4.0).round()/4.0;}
        let value=((d.rect.bottom()-p.y)/d.rect.height()).clamp(0.0,1.0);
        if let Some(points)=t.auto.get_mut(d.k) {if d.index<points.len() {
            let left=if d.index>0 {points[d.index-1].0+1e-6}else{lo};let right=if d.index+1<points.len(){points[d.index+1].0-1e-6}else{hi};
            if left<=right {points[d.index]=(x.clamp(left,right),value);}
        }}
        self.status=format!("{} volume: {:.0}% · Ctrl+Z to undo",STEM_N[d.k-5],value*100.0);
    }
}
