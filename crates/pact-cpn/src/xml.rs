//! CPN Tools 4.x XML（format 6）序列化。

use super::{escape, Builder, Direction};

pub(crate) fn render(b: &Builder) -> String {
    let mut s = String::new();
    s.push_str("<?xml version='1.0' encoding='UTF-8'?>\n");
    s.push_str(
        "<!DOCTYPE workspaceElements PUBLIC \"-//CPN//DTD CPNXML 1.0//EN\" \"http://cpntools.org/DTD/6/cpn.dtd\">\n",
    );
    s.push_str("<workspaceElements>\n");
    s.push_str("<generator tool='PACT' version='0.1.0' format='6'/>\n");
    s.push_str("<cpnet>\n");

    let mut sub = b.next_id;
    let mut sid = || {
        sub += 1;
        format!("ID{sub}")
    };

    // ---------- 全局声明 ----------
    s.push_str("<globbox>\n");

    let block = sid();
    s.push_str(&format!("<block id='{block}'>\n"));
    s.push_str("<id>Standard declarations</id>\n");
    for (name, layout, body) in [
        ("UNIT", "colset UNIT = unit;", "<unit/>"),
        ("BOOL", "colset BOOL = bool;", "<bool/>"),
        ("INT", "colset INT = int;", "<int/>"),
        ("STRING", "colset STRING = string;", "<string/>"),
    ] {
        let cid = sid();
        s.push_str(&format!(
            "<color id='{cid}'><id>{name}</id>{body}<layout>{layout}</layout></color>\n"
        ));
    }
    s.push_str("</block>\n");

    let block = sid();
    s.push_str(&format!("<block id='{block}'>\n"));
    s.push_str("<id>PACT declarations</id>\n");

    for (name, layout, body) in [
        (
            "PACT_TOKENID",
            "colset PACT_TOKENID = string;",
            "<string/>".to_string(),
        ),
        (
            "PACT_PARTICIPANT",
            "colset PACT_PARTICIPANT = string;",
            "<string/>".to_string(),
        ),
        (
            "PACT_THINGREF",
            "colset PACT_THINGREF = string;",
            "<string/>".to_string(),
        ),
        (
            "PACT_TIMESTAMP",
            "colset PACT_TIMESTAMP = int;",
            "<int/>".to_string(),
        ),
        (
            "PACT_PRECONDREFS",
            "colset PACT_PRECONDREFS = list PACT_TOKENID;",
            "<list><id>PACT_TOKENID</id></list>".to_string(),
        ),
        (
            "PACT_COMPL",
            "colset PACT_COMPL = int;",
            "<int/>".to_string(),
        ),
        (
            "PACT_TOKEN",
            "colset PACT_TOKEN = product PACT_TOKENID * PACT_PARTICIPANT * PACT_THINGREF * PACT_PRECONDREFS * PACT_TIMESTAMP;",
            "<product><id>PACT_TOKENID</id><id>PACT_PARTICIPANT</id><id>PACT_THINGREF</id><id>PACT_PRECONDREFS</id><id>PACT_TIMESTAMP</id></product>".to_string(),
        ),
        (
            "PACT_ARCHIVE",
            "colset PACT_ARCHIVE = product PACT_TOKEN * PACT_PARTICIPANT * PACT_TIMESTAMP;",
            "<product><id>PACT_TOKEN</id><id>PACT_PARTICIPANT</id><id>PACT_TIMESTAMP</id></product>".to_string(),
        ),
    ] {
        let cid = sid();
        s.push_str(&format!(
            "<color id='{cid}'><id>{name}</id>{body}<layout>{layout}</layout></color>\n"
        ));
    }

    // 变量：x 为目标 token，x1.. 为生产前驱，y1.. 为条件见证。
    let mut vars = vec!["x".to_string()];
    for i in 1..=b.max_predecessors {
        vars.push(format!("x{i}"));
    }
    for i in 1..=b.max_witnesses {
        vars.push(format!("y{i}"));
    }
    for var in vars {
        let vid = sid();
        s.push_str(&format!(
            "<var id='{vid}'><type><id>PACT_TOKEN</id></type><id>{var}</id><layout>var {var}: PACT_TOKEN;</layout></var>\n"
        ));
    }

    let ml = r#"val pact_counter = ref 0;

fun pact_next_id () =
  let val n = !pact_counter + 1
  in pact_counter := n; "t" ^ Int.toString n end;

fun pact_now () = 0;"#;
    let mid = sid();
    s.push_str(&format!(
        "<ml id='{mid}'><layout>{}</layout>\n{}\n</ml>\n",
        escape(ml),
        escape(ml)
    ));
    s.push_str("</block>\n");
    s.push_str("</globbox>\n");

    // ---------- 页面 ----------
    let page = sid();
    s.push_str(&format!("<page id='{page}'>\n"));
    s.push_str(&format!("<pageattr name='{}'/>\n", escape(&b.page_name)));

    for p in &b.places {
        render_place(&mut s, p, &mut sid);
    }
    for t in &b.trans {
        render_trans(&mut s, t, &mut sid);
    }
    for a in &b.arcs {
        render_arc(&mut s, a, &mut sid);
    }

    s.push_str("</page>\n");
    s.push_str("<instances/>\n");
    s.push_str("<options/>\n");
    s.push_str("<binders/>\n");
    s.push_str("<monitorblock name='Monitors'/>\n");
    s.push_str("<IndexNode expanded='false'/>\n");
    s.push_str("</cpnet>\n");
    s.push_str("</workspaceElements>\n");
    s
}

fn objatts(s: &mut String, x: f64, y: f64) {
    s.push_str(&format!("<posattr x='{x:.6}' y='{y:.6}'/>\n"));
    s.push_str("<fillattr colour='White' pattern='' filled='false'/>\n");
    s.push_str("<lineattr colour='Black' thick='1' type='Solid'/>\n");
    s.push_str("<textattr colour='Black' bold='false'/>\n");
}

fn render_place(s: &mut String, p: &super::PlaceDef, sid: &mut impl FnMut() -> String) {
    let type_id = sid();
    let init_id = sid();
    s.push_str(&format!("<place id='{}'>\n", p.id));
    objatts(s, p.x, p.y);
    s.push_str(&format!("<text>{}</text>\n", escape(&p.name)));
    s.push_str("<ellipse w='90.000000' h='40.000000'/>\n");
    s.push_str(&format!("<type id='{type_id}'>\n"));
    s.push_str("<posattr x='0.000000' y='0.000000'/>\n");
    s.push_str("<fillattr colour='White' pattern='Solid' filled='false'/>\n");
    s.push_str("<lineattr colour='Black' thick='0' type='Solid'/>\n");
    s.push_str("<textattr colour='Black' bold='false'/>\n");
    s.push_str(&format!(
        "<text tool='CPN Tools' version='4.0.1'>{}</text>\n",
        escape(&p.color)
    ));
    s.push_str("</type>\n");
    s.push_str("<marking x='0.000000' y='0.000000' hidden='false'/>\n");
    s.push_str(&format!("<initmark id='{init_id}'>\n"));
    s.push_str("<posattr x='0.000000' y='0.000000'/>\n");
    s.push_str("<fillattr colour='White' pattern='Solid' filled='false'/>\n");
    s.push_str("<lineattr colour='Black' thick='0' type='Solid'/>\n");
    s.push_str("<textattr colour='Black' bold='false'/>\n");
    s.push_str(&format!(
        "<text tool='CPN Tools' version='4.0.1'>{}</text>\n",
        escape(&p.init)
    ));
    s.push_str("</initmark>\n");
    s.push_str("</place>\n");
}

fn render_trans(s: &mut String, t: &super::TransDef, sid: &mut impl FnMut() -> String) {
    let cond_id = sid();
    let time_id = sid();
    let code_id = sid();
    s.push_str(&format!("<trans id='{}' explicit='false'>\n", t.id));
    objatts(s, t.x, t.y);
    s.push_str(&format!("<text>{}</text>\n", escape(&t.name)));
    s.push_str("<box w='100.000000' h='40.000000'/>\n");
    s.push_str("<binding x='7.200000' y='-3.000000'/>\n");
    s.push_str(&format!("<cond id='{cond_id}'>\n"));
    objatts(s, 0.0, 0.0);
    s.push_str(&format!(
        "<text tool='CPN Tools' version='4.0.1'>{}</text>\n",
        escape(&t.guard)
    ));
    s.push_str("</cond>\n");
    s.push_str(&format!("<time id='{time_id}'>\n"));
    objatts(s, 0.0, 0.0);
    s.push_str("<text tool='CPN Tools' version='4.0.1'></text>\n");
    s.push_str("</time>\n");
    s.push_str(&format!("<code id='{code_id}'>\n"));
    objatts(s, 0.0, 0.0);
    s.push_str("<text tool='CPN Tools' version='4.0.1'></text>\n");
    s.push_str("</code>\n");
    s.push_str("</trans>\n");
}

fn render_arc(s: &mut String, a: &super::ArcDef, sid: &mut impl FnMut() -> String) {
    let annot_id = sid();
    let orientation = match a.dir {
        Direction::PtoT => "PtoT",
        Direction::TtoP => "TtoP",
    };
    s.push_str(&format!(
        "<arc id='{}' orientation='{orientation}' order='1'>\n",
        a.id
    ));
    objatts(s, 0.0, 0.0);
    s.push_str("<arrowattr headsize='1.200000' currentcyckle='2'/>\n");
    s.push_str(&format!("<transend idref='{}'/>\n", a.to_or_from_trans()));
    s.push_str(&format!("<placeend idref='{}'/>\n", a.to_or_from_place()));
    s.push_str(&format!("<annot id='{annot_id}'>\n"));
    objatts(s, 0.0, 0.0);
    s.push_str(&format!(
        "<text tool='CPN Tools' version='4.0.1'>{}</text>\n",
        escape(&a.expr)
    ));
    s.push_str("</annot>\n");
    s.push_str("</arc>\n");
}

impl super::ArcDef {
    /// 无论 PtoT 还是 TtoP，`transend` 总引用迁移端。
    fn to_or_from_trans(&self) -> &str {
        match self.dir {
            Direction::PtoT => &self.to,
            Direction::TtoP => &self.from,
        }
    }

    fn to_or_from_place(&self) -> &str {
        match self.dir {
            Direction::PtoT => &self.from,
            Direction::TtoP => &self.to,
        }
    }
}
