//! classification profiler
//! 数据分类探针
//! 根据数据分类结果来计算分类成绩
//! 数据分类结果为Excel文档，格式为
//!
//! |class1|class2|class3|...|classn|数据库|表|字段|
//! |---|---|---|---|---|----|---|----|
//! |c1|c2|c3|...|cn|db1|tb1|field1|
//!
//! 探针的同级目录应设置为
//! ```bash
//! - classification_profiler
//!   - cls
//!   - industry
//!     - bank
//!       - 银行-结果.xlsx
//!       - 银行-模版.xlsx
//! ```
//!
//! 部署探针的过程
//!
//! `deploy.sh -i <bank> -h`
//! 1. 使用cls程序，将对应行业的数据分类结果文件加密
//! 2. 将cls程序，对应行业的加密后数据分类结果文件，模版文件打包，并删除本地加密文件
//! 3. 将压缩包发送到指定主机位置，sftp
//! 4. 远程执行命令，解压压缩包并且验证结果文件
//!
//! 探针功能
//! 1. cls -a <分类结果.xlsx>，对比标准答案，生成分类成绩，即总的正确率以及在各大类下的正确率
//! 2. cls -e <分类结果.xlsx>，将分类结果加密，生成加密文件enc

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    error::Error,
    fmt::Display,
    fs,
    io::{BufReader, Cursor, Read, Write},
    path::PathBuf,
};

use aes_gcm::{
    aead::{Aead, OsRng},
    AeadCore, Aes256Gcm, Key, KeyInit,
};
use anyhow::Context;
use calamine::{open_workbook, open_workbook_from_rs, DataType, Reader, Xlsx};
use clap::{arg, value_parser, Command};
use serde::{ser::SerializeTupleStruct, Serialize};

const ENC_FILE_PATH: &str = "./fix_e";
const ENC_KEY: &[u8; 32] = &[
    232, 222, 212, 202, 166, 177, 188, 199, 87, 34, 44, 10, 102, 1, 9, 0, 32, 22, 22, 20, 136, 177,
    128, 199, 87, 32, 44, 10, 102, 2, 4, 6,
];
const CLASSI_SHEET: &str = "Sheet 1";
const NONCE_LEN: usize = 96 / 8;

#[derive(Serialize, Debug, Default)]
struct DiffUnit {
    classis: Vec<String>,
    field: String,
    field_exist: bool,
}

type DiffResult = Vec<DiffUnit>;

fn claussi_report(r: &DiffResult) -> anyhow::Result<()> {
    let json_res = serde_json::to_string_pretty(&r)?;

    let total = r.len() as i32;
    let mut match_classi = 0;
    let mut group_statistic = HashMap::<String, (i32, i32)>::new();
    for unit in r {
        let first_classi = unit.classis[0].clone();
        let cal_u = if unit.field_exist { 1 } else { 0 };
        match_classi += cal_u;
        group_statistic
            .entry(first_classi)
            .and_modify(|e| {
                e.0 += 1;
                e.1 += cal_u;
            })
            .or_insert((1, cal_u));
    }

    let ratio = match_classi as f64 / total as f64;
    println!("total classification accuracy: {:.2}%", ratio * 100f64);

    for (k, v) in group_statistic {
        println!(
            "classification [{}] accuracy: {:.2}%",
            k,
            v.1 as f64 / v.0 as f64 * 100f64
        );
    }

    Ok(())
}

#[derive(Debug)]
struct ClassiError {
    msg: &'static str,
}

impl ClassiError {
    fn new(msg: &'static str) -> Self {
        Self { msg }
    }
}

impl Display for ClassiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "classification error: {}", self.msg)
    }
}

impl Error for ClassiError {}

type Database = String;
type Table = String;
type Field = String;

#[derive(Debug, PartialEq, Eq, Clone, Default, Hash)]
struct FieldMeta(Database, Table, Field);

impl Display for FieldMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}-{}", self.0, self.1, self.2)
    }
}

impl Serialize for FieldMeta {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut ser = serializer.serialize_tuple_struct("field", 3)?;
        ser.serialize_field(&self.0)?;
        ser.serialize_field(&self.1)?;
        ser.serialize_field(&self.2)?;
        ser.end()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum ClassiVal {
    Root,
    Classi(String),
    Field(FieldMeta),
}

impl Display for ClassiVal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassiVal::Root => write!(f, "root"),
            ClassiVal::Classi(ref s) => write!(f, "classi({})", s),
            ClassiVal::Field(ref dtf) => write!(f, "field({})", dtf),
        }
    }
}

struct ClassiNode {
    val: ClassiVal,
    subs: Option<Vec<ClassiNode>>,
}

impl From<&ClassiVal> for ClassiNode {
    fn from(value: &ClassiVal) -> Self {
        let val = value.clone();
        Self { val, subs: None }
    }
}

impl ClassiNode {
    fn new(val: ClassiVal) -> Self {
        Self { val, subs: None }
    }

    fn find_node(&self, val: &ClassiVal) -> Option<&ClassiNode> {
        if self.val == *val {
            Some(self)
        } else {
            match self.subs {
                Some(ref subs) => {
                    for sub_node in subs {
                        if let Some(n) = sub_node.find_node(val) {
                            return Some(n);
                        }
                    }
                    None
                }
                None => None,
            }
        }
    }

    fn add_node(&mut self, sup_val: &ClassiVal, val: &ClassiVal) -> Result<(), ClassiError> {
        if self.val == *sup_val {
            let t_node = ClassiNode::from(val);
            match self.subs {
                Some(ref mut subs) => {
                    for e in subs.iter() {
                        if e.val == *val {
                            return Err(ClassiError::new("the node exists"));
                        }
                    }
                    subs.push(t_node);
                }
                None => {
                    let new_nodes = vec![t_node];
                    self.subs = Some(new_nodes);
                }
            }
            Ok(())
        } else {
            match self.subs {
                Some(ref mut subs) => {
                    let mut is_add = false;
                    for e in subs {
                        match e.add_node(sup_val, val) {
                            Ok(_) => {
                                is_add = true;
                                break;
                            }
                            Err(e) => {
                                if e.msg == "the node exists" {
                                    return Err(e);
                                } else {
                                    continue;
                                }
                            }
                        }
                    }
                    if !is_add {
                        Err(ClassiError::new("the super node does not found"))
                    } else {
                        Ok(())
                    }
                }
                None => Err(ClassiError::new("the super node does not found")),
            }
        }
    }

    fn to_string(&self, space: usize) -> String {
        const INDENT: &str = "  ";
        let mut res = String::new();
        match self.val {
            ClassiVal::Root => {
                if let Some(sub) = &self.subs {
                    for e in sub {
                        res.push_str(&e.to_string(space));
                    }
                }
            }
            ClassiVal::Classi(ref inner) => {
                res.push_str((INDENT.repeat(space) + inner.as_str() + "\n").as_str());
                if let Some(sub) = &self.subs {
                    for e in sub {
                        res.push_str(&e.to_string(space + 1));
                    }
                }
            }
            ClassiVal::Field(ref dtf) => {
                res.push_str((INDENT.repeat(space) + dtf.to_string().as_str() + "\n").as_str());
            }
        }

        res
    }
}

struct ClassiTree {
    root: ClassiNode,
}

impl ClassiTree {
    fn new() -> Self {
        ClassiTree {
            root: ClassiNode::new(ClassiVal::Root),
        }
    }

    fn find_node(&self, val: &ClassiVal) -> Option<&ClassiNode> {
        self.root.find_node(val)
    }

    fn add_node(&mut self, classis: &[&str], field: FieldMeta) -> Result<(), ClassiError> {
        let l = classis.len();
        match l.cmp(&1usize) {
            Ordering::Greater => {
                let _ = self.root.add_node(
                    &ClassiVal::Root,
                    &ClassiVal::Classi(String::from(classis[0])),
                );

                for win in classis.windows(2) {
                    match self.root.add_node(
                        &ClassiVal::Classi(String::from(win[0])),
                        &ClassiVal::Classi(String::from(win[1])),
                    ) {
                        Ok(_) => continue,
                        Err(e) => {
                            if e.msg == "the node exists" {
                                continue;
                            } else {
                                return Err(ClassiError::new("failed to add classification level"));
                            }
                        }
                    }
                }

                self.root.add_node(
                    &ClassiVal::Classi(String::from(classis[classis.len() - 1])),
                    &ClassiVal::Field(field),
                )
            }
            Ordering::Less => Err(ClassiError::new("classification levels must be provided")),
            Ordering::Equal => {
                let _ = self.root.add_node(
                    &ClassiVal::Root,
                    &ClassiVal::Classi(String::from(classis[0])),
                );
                self.root.add_node(
                    &ClassiVal::Classi(String::from(classis[0])),
                    &ClassiVal::Field(field),
                )
            }
        }
    }

    fn all_leaves(&self) -> Vec<Vec<&ClassiNode>> {
        let mut res = Vec::new();

        let mut cur_q = Vec::<&ClassiNode>::new();
        if let Some(ref subs) = self.root.subs {
            for sub in subs {
                ClassiTree::_collect_leave(sub, &mut cur_q, &mut res);
                cur_q.clear();
            }
        }
        res
    }

    fn _collect_leave<'a>(
        node: &'a ClassiNode,
        cur_q: &mut Vec<&'a ClassiNode>,
        res: &mut Vec<Vec<&'a ClassiNode>>,
    ) {
        cur_q.push(node);
        if let Some(ref subs) = node.subs {
            for sub in subs {
                ClassiTree::_collect_leave(sub, cur_q, res);
            }
            cur_q.pop();
        } else {
            res.push(cur_q.clone());
            cur_q.pop();
        }
    }

    /// 和另一棵分类结果树做对比，生成对比结果
    fn diff(&self, other: &ClassiTree) -> DiffResult {
        let all_fields = self.all_leaves();
        let mut res = Vec::new();
        for field in all_fields {
            let mut t_q = Vec::new();
            let mut is_found = true;
            for seg in &field {
                match other.find_node(&seg.val) {
                    Some(node) => match node.val {
                        ClassiVal::Classi(ref classi) => t_q.push(classi.clone()),
                        ClassiVal::Field(ref field) => {
                            let unit = DiffUnit {
                                classis: t_q.clone(),
                                field: field.to_string(),
                                field_exist: true,
                            };
                            res.push(unit);
                        }
                        _ => (),
                    },
                    None => is_found = false,
                }
            }
            if !is_found {
                let unit = DiffUnit {
                    classis: field[0..field.len()]
                        .iter()
                        .map(|n| match &n.val {
                            ClassiVal::Classi(classi) => classi.clone(),
                            _ => String::new(),
                        })
                        .collect(),
                    field: field.last().unwrap().val.to_string(),
                    field_exist: false,
                };
                res.push(unit);
            }
        }

        res
    }
}

impl Display for ClassiTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.root.to_string(0).trim())
    }
}

fn new_workbook_from_file(file_path: &PathBuf) -> anyhow::Result<Xlsx<BufReader<fs::File>>> {
    let workbook: Xlsx<_> = open_workbook(file_path)?;
    Ok(workbook)
}

fn new_workbook_from_bytes(bytes: &Vec<u8>) -> anyhow::Result<Xlsx<Cursor<&Vec<u8>>>> {
    let cursor = Cursor::new(bytes);
    let workbook: Xlsx<_> = open_workbook_from_rs(cursor)?;
    Ok(workbook)
}

/// 读取分类结果，转化为分类树
fn read_classi_result(file_path: &PathBuf, is_enc: bool) -> anyhow::Result<ClassiTree> {
    let sheet = if is_enc {
        let decrypt_result = decrypt_file(file_path).with_context(|| {
            format!(
                "failed to decrypt the standard answer file [{}]",
                file_path.to_string_lossy()
            )
        })?;
        let mut workbook = new_workbook_from_bytes(&decrypt_result)?;
        workbook
            .worksheet_range(CLASSI_SHEET)
            .with_context(|| format!("failed to open the sheet [{}]", CLASSI_SHEET))?
    } else {
        let mut workbook = new_workbook_from_file(file_path)?;
        workbook
            .worksheet_range(CLASSI_SHEET)
            .with_context(|| format!("failed to open the sheet [{}]", CLASSI_SHEET))?
    };

    let headers = sheet
        .headers()
        .ok_or(ClassiError::new("failed to retrieve the header"))?;

    let mut classi_counter = 0;
    for head in &headers {
        if head == "数据库名称" {
            break;
        } else {
            classi_counter += 1;
        }
    }

    assert_ne!(
        classi_counter, 0,
        "the number of classification levels cannot be 0"
    );
    assert_eq!(headers.len(), classi_counter + 3, "header count error");

    let maybe_row_len = sheet.get_size().0;
    let range = sheet.range((1, 0), (maybe_row_len as u32, classi_counter as u32 + 2));

    let mut tree = ClassiTree::new();
    let mut field_filter = HashSet::<FieldMeta>::new();

    for row in range.rows() {
        if row.len() != classi_counter + 3 {
            break;
        } else {
            if row.is_empty() || row.first().unwrap().is_empty() {
                continue;
            }

            let mut lvls = vec![];
            for i in 0..classi_counter {
                lvls.push(row.get(i).unwrap().get_string().unwrap());
            }
            let db = String::from(row.get(classi_counter).unwrap().get_string().unwrap());
            let tb = String::from(row.get(classi_counter + 1).unwrap().get_string().unwrap());
            let fd = String::from(row.get(classi_counter + 2).unwrap().get_string().unwrap());
            let field_meta = FieldMeta(db, tb, fd);
            if field_filter.contains(&field_meta) {
                return Err(ClassiError::new("duplicated field detected").into());
            } else {
                field_filter.insert(field_meta.clone());
            }

            tree.add_node(&lvls, field_meta)?;
        }
    }

    Ok(tree)
}

/// 读取结果并将结果文件加密转存
fn encrypt_file(ori_file: &PathBuf, enc_file: &PathBuf) -> anyhow::Result<()> {
    let ori_file = fs::read(ori_file)?;
    let key: &Key<Aes256Gcm> = ENC_KEY.into();
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let cipher = Aes256Gcm::new(key);
    let cipher_content = cipher
        .encrypt(&nonce, ori_file.as_ref())
        .map_err(|e| anyhow::Error::msg(e.to_string()))?;
    let mut enc_file = fs::File::create(enc_file)?;
    let nonce_len = enc_file.write(&nonce)?;
    if nonce_len != nonce.len() {
        return Err(anyhow::Error::msg("failed to write the nonce"));
    }
    let _ = enc_file.write(&cipher_content)?;
    Ok(())
}

/// 读取加密文件内容
fn decrypt_file(enc_file: &PathBuf) -> anyhow::Result<Vec<u8>> {
    let key: &Key<Aes256Gcm> = ENC_KEY.into();
    let cipher = Aes256Gcm::new(key);

    let mut enc_file = fs::File::open(enc_file)?;
    let mut buf = Vec::new();
    let _ = enc_file.read_to_end(&mut buf)?;
    let nonce = &buf[..NONCE_LEN];
    let cipher_content = &buf[NONCE_LEN..];

    let plain_content = cipher
        .decrypt(nonce.into(), cipher_content)
        .map_err(|e| anyhow::Error::msg(e.to_string()))?;
    Ok(plain_content)
}

fn main() -> anyhow::Result<()> {
    let matches = Command::new("cls_profiler")
        .about("数据分类探针")
        .version("1.0.0")
        .args([
            arg!(answer: -a --answer <FILE> "指定分类结果文件的路径")
                .value_parser(value_parser!(PathBuf)),
            arg!(encrypt: -e --encrypt <FILE> "指定要加密的分类结果文件的路径")
                .value_parser(value_parser!(PathBuf)),
        ])
        .arg_required_else_help(true)
        .get_matches();

    if let Some(ef) = matches.get_one::<PathBuf>("encrypt") {
        encrypt_file(ef, &PathBuf::from(ENC_FILE_PATH))?;
    }

    if let Some(af) = matches.get_one::<PathBuf>("answer") {
        let solution_file = PathBuf::from(ENC_FILE_PATH);
        let solution = read_classi_result(&solution_file, true)?;
        let answer = read_classi_result(af, false)?;
        let diff_res: DiffResult = solution.diff(&answer);
        claussi_report(&diff_res)?;
    }

    Ok(())
}
