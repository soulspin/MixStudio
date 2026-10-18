//! Direct manipulation of a clip's independent edge fade shapes.
use super::*;
pub(super) fn draw(ui:&egui::Ui,tp:&egui::Painter,ti:usize,t:&Track,wave:Rect,vis:Rect,bx:&impl Fn(f64)->f32)->Option<TlDrag> {
    let mut action=None;
    for left in [true,false] {
        let side=if left {0}else{1};let length=if left {t.fade_in}else{t.fade_out};
        let edge=if left {t.vis_start(0.0)}else{t.end(0.0)};
        let at=|p:f64| if left {edge+length*p}else{edge-length*p};
        if length>0.0 {
            let points=(0..=32).map(|i| {let p=i as f64/32.0;pos2(bx(at(p)),wave.bottom()-crate::fades::value(p,t.fade_curve[side]) as f32*wave.height())}).collect();
            tp.with_clip_rect(vis.intersect(wave)).add(egui::Shape::line(points,Stroke::new(1.5_f32,C::WHITE)));
            let mid=pos2(bx(at(0.5)),wave.bottom()-crate::fades::value(0.5,t.fade_curve[side]) as f32*wave.height());
            let hr=Rect::from_center_size(mid,vec2(10.0,10.0));
            if vis.contains(mid) {
                let r=ui.interact(hr,ui.id().with(("fade_shape",ti,left)),Sense::click_and_drag());
                tp.circle_filled(mid,3.5,if r.hovered()||r.dragged(){ACCENT}else{C::WHITE});
                if r.double_clicked(){action=Some(TlDrag::Curve{ti,left,wave,reset:true});}
                else if r.drag_started(){action=Some(TlDrag::Curve{ti,left,wave,reset:false});}
                r.on_hover_text("Drag up/down to bend the fade. Double-click for a linear fade.");
            }
        }
        let center=pos2(bx(at(1.0))+if left {4.0}else{-4.0},wave.top()+4.0);
        if !vis.contains(center){continue;}
        let hr=Rect::from_center_size(center,vec2(8.0,8.0));
        let r=ui.interact(hr,ui.id().with(("fade",ti,left)),Sense::drag());
        tp.rect_filled(hr,1.5,if r.hovered()||r.dragged(){ACCENT}else{C::WHITE});
        if r.drag_started(){action=Some(TlDrag::Fade{ti,left});}
        r.on_hover_text(if left {"Drag horizontally: fade-in length"}else{"Drag horizontally: fade-out length"});
    }
    action
}
