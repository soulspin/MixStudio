//! Library navigation derived only from the index; never probes audio drives on the UI thread.
use super::{Entry,Filter,SortCol};
use std::path::Path;
#[derive(Clone,Debug)]
pub struct FolderNode {pub path:String,pub name:String,pub count:usize,pub children:Vec<FolderNode>}
impl FolderNode {
    fn new(path:&Path)->Self {Self{path:path.display().to_string(),name:path.file_name().map(|n|n.to_string_lossy().into_owned()).unwrap_or_else(||path.display().to_string()),count:0,children:vec![]}}
    fn insert(&mut self,parts:&[String]) {
        self.count+=1;
        if let Some((name,rest))=parts.split_first() {
            let key=(name.to_lowercase(),name);
            let i=match self.children.binary_search_by(|c|(c.name.to_lowercase(),&c.name).cmp(&key)) {Ok(i)=>i,Err(i)=>{self.children.insert(i,Self::new(&Path::new(&self.path).join(name)));i}};
            self.children[i].insert(rest);
        }
    }
}
pub fn folder_tree(roots:&[String],entries:&[Entry])->Vec<FolderNode> {
    roots.iter().map(|root| {
        let mut node=FolderNode::new(Path::new(root));let mut seen=std::collections::HashSet::new();
        for e in entries {if !seen.insert(&e.path){continue;}if let Some(parent)=Path::new(&e.path).parent() {if let Ok(relative)=parent.strip_prefix(root) {
            let parts:Vec<_>=relative.components().map(|c|c.as_os_str().to_string_lossy().into_owned()).collect();node.insert(&parts);
        }}}
        node
    }).collect()
}
pub fn folder_view(entries:&[Entry],filter:&Filter<'_>,sort:SortCol,asc:bool,folder:Option<&str>)->Vec<usize> {
    let mut view=super::view(entries,filter,sort,asc);
    if let Some(folder)=folder {view.retain(|&i|Path::new(&entries[i].path).starts_with(folder));}
    view
}
