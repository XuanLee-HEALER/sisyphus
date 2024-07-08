use std::{
    cmp::Ordering,
    error::Error,
    fmt::{Debug, Display},
    fs,
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};

use aes_gcm::{
    aead::{Aead, Nonce},
    Aes256Gcm, Key, KeyInit,
};
use anyhow::Context;
use calamine::{open_workbook, DataType, Reader, Xlsx};

const ENC_FILE_PATH: &str = "./enc";
const ENC_KEY: &[u8; 32] = &[
    232, 222, 212, 202, 166, 177, 188, 199, 87, 34, 44, 10, 102, 1, 9, 0, 32, 22, 22, 20, 136, 177,
    128, 199, 87, 32, 44, 10, 102, 2, 4, 6,
];
const CLASSI_SHEET: &str = "Sheet 1";

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

#[derive(Debug, PartialEq, Eq, Clone)]
struct FieldMeta(Database, Table, Field);

impl Display for FieldMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}-{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum ClassiVal {
    Root,
    Classi(String),
    Field(FieldMeta),
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
}

impl Display for ClassiTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.root.to_string(0).trim())
    }
}

/// 在工作目录生成一个Excel结果模版
fn generate_result_tmpl() -> Result<(), io::Error> {
    Ok(())
}

fn read_classi_result(file_path: &PathBuf) -> anyhow::Result<String> {
    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .with_context(|| format!("failed to open the excel file {:?}", file_path))?;
    let sheet = workbook
        .worksheet_range(CLASSI_SHEET)
        .with_context(|| format!("failed to open the sheet [{}]", CLASSI_SHEET))?;
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

    'out: for row in range.rows() {
        if row.len() != classi_counter + 3 {
            break;
        } else {
            for v in row {
                if v.is_empty() || !v.is_string() {
                    break 'out;
                }
            }

            let mut lvls = vec![];
            for i in 0..classi_counter {
                lvls.push(row.get(i).unwrap().get_string().unwrap());
            }
            let db = String::from(row.get(classi_counter).unwrap().get_string().unwrap());
            let tb = String::from(row.get(classi_counter + 1).unwrap().get_string().unwrap());
            let fd = String::from(row.get(classi_counter + 2).unwrap().get_string().unwrap());
            // println!("parameters:\n{:?} {}", lvls, FieldMeta(db, tb, fd));
            tree.add_node(&lvls, FieldMeta(db, tb, fd))?;
        }
    }

    println!("{}", tree);

    Ok(String::from("value"))
}

/// 读取结果并将结果文件加密转存
fn encrypt_result(result_file: &PathBuf, enc_file: Option<&PathBuf>) -> Result<(), io::Error> {
    // opens a new workbook

    // let mut enc_path = &PathBuf::from(ENC_FILE_PATH);
    // if let Some(dst) = enc_file {
    //     enc_path = dst;
    // }
    // let mut enc_file = fs::File::create(enc_path)?;
    // let key: &Key<Aes256Gcm> = ENC_KEY.into();
    // let cipher = Aes256Gcm::new(key);

    // 'out: for row in range.rows() {
    //     if !row.is_empty() {
    //         match row.first() {
    //             Some(Data::String(_)) => {
    //                 let cols: Vec<String> = row.iter().map_while(|d| d.as_string()).collect();
    //                 let line = cols.join(",");
    //                 let plain_text = line.as_bytes();
    //                 let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    //                 match cipher.encrypt(&nonce, plain_text) {
    //                     Ok(mut cipher_text) => {
    //                         cipher_text.push(b'|');
    //                         cipher_text.append(nonce.to_vec().as_mut());
    //                         cipher_text.push(b'\n');
    //                         let _ = enc_file.write(&cipher_text)?;
    //                     }
    //                     Err(e) => {
    //                         return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
    //                     }
    //                 }
    //             }
    //             _ => break 'out,
    //         }
    //     } else {
    //         break;
    //     }
    // }

    Ok(())
}

fn read_enc_file(file_path: &PathBuf) -> Result<Vec<String>, io::Error> {
    let enc_file = fs::File::open(file_path)?;
    let enc_file = BufReader::new(enc_file);

    let key: &Key<Aes256Gcm> = ENC_KEY.into();
    let cipher = Aes256Gcm::new(key);
    let mut rec = Vec::new();

    for line in enc_file.lines() {
        let line = line?;
        let segs: Vec<&str> = line.split('|').collect();
        if segs.len() != 2 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "inline delimiter not found",
            ));
        } else {
            let encrypt_text: &[u8] = segs[0].as_bytes();
            let nonce: &[u8] = segs[1].as_bytes();
            let nonce: &Nonce<Aes256Gcm> = nonce.into();
            match cipher.decrypt(nonce, encrypt_text) {
                Ok(decrypt_text) => match String::from_utf8(decrypt_text) {
                    Ok(dec_str) => {
                        rec.push(dec_str);
                    }
                    Err(e) => {
                        return Err(io::Error::new(io::ErrorKind::Other, e));
                    }
                },
                Err(e) => {
                    return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
                }
            }
        }
    }

    Ok(rec)
}

struct ClassificationReport {}

/// 读取Excel结果并与正确答案比对，生成一个分类结果报告
fn generate_report() -> ClassificationReport {
    ClassificationReport {}
}

fn main() -> anyhow::Result<()> {
    // let enc_file = PathBuf::from("./enc.txt");
    // let ori = read_enc_file(&enc_file).map_err(ClsError::OtherError)?;
    // for o in ori {
    //     println!("original text: {}", o);
    // }
    let _ = read_classi_result(&PathBuf::from("./test.xlsx")).expect("error occurred");

    // let matches = Command::new("cls_profiler")
    // .about("分类探针应用")
    // .version("1.0.0")
    // .subcommand(
    //     Command::new("restmpl")
    //     .about("生成分类结果模版")
    //     .arg(arg!(-t --tmpl [tmpl_path] "，tmpl_path为模版的生成路径，需要一个程序有写入权限的路径，不填默认为工作目录"))
    // )
    // .subcommand(
    //     Command::new("encrypt")
    //     .about("加密结果文件")
    //     .arg(arg!(-e --enc <result> "result为需要加密的结果文件的具体路径，文件格式为xlsx").required(true).value_parser(value_parser!(PathBuf)))
    //     .arg(arg!(-p [enc_result_path] "enc_result_path为目标文件的生成路径，需要一个程序有写入权限的路径，不填默认为工作目录")),
    // )
    // .subcommand(
    //     Command::new("report")
    //     .about("生成分类结果报告")
    //     .arg(arg!(-r <result_path> "result_path是分类结果").required(true))
    //     .arg(arg!(-e <encrypted_result_path> "encrypted_result_path是加密的正确分类结果").required(true))
    //     .arg(arg!(-o <out_fmt> "out_fmt指定输出目标，CONSOLE为标准输出，PDF为pdf文件").required(true).value_parser(["CONSOLE", "PDF"]))
    //     .arg(arg!(-p [report_path] "report_path指定一个有写入权限的目录，默认为工作目录")),
    // )
    // .subcommand_required(true)
    // .get_matches();

    // if let Some(enc) = matches.subcommand_matches("encrypt") {
    //     if let Some(result_path) = enc.get_one::<PathBuf>("enc") {
    //         let enc_path = enc
    //             .get_one("enc_result_path")
    //             .map(|ep: &String| PathBuf::from(ep));
    //         return match encrypt_result(result_path, enc_path.as_ref()) {
    //             Ok(()) => Ok(()),
    //             Err(e) => Err(Box::new(e)),
    //         };
    //     } else {
    //         return Err(Box::new(ClsError::InternalError));
    //     }
    // }

    // if let Some(report) = matches.subcommand_matches("report") {
    //     if let Some(result_path) = report.get_one::<String>("result_path") {
    //         println!("result_path => {}", result_path)
    //     } else {
    //         return Err(Box::new(ClsError::InternalError));
    //     }
    // }

    Ok(())
}
