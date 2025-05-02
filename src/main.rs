//use std::fs::File;
//use std::io::Read;
use std::fs;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, sync::Mutex};
use chrono::{Datelike, Duration, DurationRound as _, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use teloxide::{prelude::*, types::ParseMode};
use sqlx::{migrate::MigrateDatabase, Executor, Pool, Row, Sqlite, SqlitePool};
use tokio::time::sleep;

#[derive(Debug)]
enum ShowError {
    Database(sqlx::Error),
    Telegram(teloxide::RequestError),
    Reqwest(reqwest::Error),
    Other(String),
}

impl std::fmt::Display for ShowError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ShowError::Database(e) => write!(f, "SQLx Error: {}", e),
            ShowError::Telegram(e) => write!(f, "Teloxide Error: {}", e),
            ShowError::Reqwest(e) => write!(f, "Reqwest Error: {}", e),
            ShowError::Other(item) => write!(f, "Other Error: {}", item),
        }
    }
}

impl From<sqlx::Error> for ShowError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

impl From<reqwest::Error> for ShowError {
    fn from(value: reqwest::Error) -> Self {
        Self::Reqwest(value)
    }
}

impl From<teloxide::RequestError> for ShowError {
    fn from(value: teloxide::RequestError) -> Self {
        Self::Telegram(value)
    }
}

impl From<String> for ShowError {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

impl ShowError {
    fn tel_err(self) -> teloxide::RequestError {
        match self {
            Self::Telegram(v) => v,
            ShowError::Database(e) => {
                teloxide::RequestError::Io(std::io::Error::other(e.to_string()))
            },
            Self::Reqwest(e) => {
                teloxide::RequestError::Io(std::io::Error::other(e.to_string()))
            },
            ShowError::Other(e) => {
                teloxide::RequestError::Io(std::io::Error::other(e))
            },
        }
    }
}


#[derive(Serialize, Deserialize, sqlx::FromRow, Clone, Debug)]
struct JjamRow {
    dates: String,
    brst: String,
    brst_cal: String,
    lunc: String,
    lunc_cal: String,
    dinr: String,
    dinr_cal: String,
    adspcfd: String,
    adspcfd_cal: String,
    sum_cal: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct JjamData {
    list_total_count: i32,
    row: RefCell<Vec<JjamRow>>,
}

enum MealType {
    Breakfast,
    Lunch,
    Dinner,
}

impl MealType {
    fn get_data<'a>(&self, jjam: &'a JjamRow) -> (&'a str, &'a str) {
        match self {
            MealType::Breakfast => (&jjam.brst, &jjam.brst_cal),
            MealType::Lunch => (&jjam.lunc, &jjam.lunc_cal),
            MealType::Dinner => (&jjam.dinr, &jjam.dinr_cal),
        }
    }
}


#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct JjamDataShell {
    DS_TB_MNDT_DATEBYMLSVC_7021: JjamData,
} 

#[allow(dead_code)]
#[derive(sqlx::FromRow, Clone, Debug)]
struct UserData {
    userid: u64,
    username: String,
    realname: String,
}

#[allow(dead_code)]
#[derive(sqlx::FromRow, Clone, Debug)]
struct TimerData {
    userid: u64,
    when: NaiveTime,
    what: String
}

const TG_TOKEN: &str = "TELOXIDE_TOKEN";
const JJAM_TOKEN: &str = "JJAM_TOKEN";
const UNIT_CODE: &str = "UNIT_CODE";
const USER_AGENT_FIREFOX: &str = "Mozilla/5.0 (X11; Linux x86_64;rv:60.0) Gecko/20100101 Firefox/81.0";

// 식사집합 순서 배열
const ORDERS: [&str; 3] = [ "본-1-2", "2-본-1", "1-2-본" ];



const HM_00_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(0, 0, 0, 0);
const HM_07_15: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(7, 15, 0, 0);
const HM_08_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(8, 0, 0, 0);
const HM_11_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(11, 0, 0, 0);
const HM_12_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(12, 0, 0, 0);
const HM_17_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(17, 0, 0, 0);
const HM_18_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(18, 0, 0, 0);
const HM_20_00: Option<NaiveTime> = NaiveTime::from_hms_nano_opt(20, 0, 0, 0);

use lazy_static::lazy_static;

lazy_static! {
    // 식사집합 순서를 두 곳에서 별도로 관리
    static ref ORDERIDX: Mutex<[usize; 2]> = Mutex::new([0, 0]);
}

fn time_now() -> chrono::DateTime<chrono::Utc> {
    chrono::offset::Utc::now() + chrono::Duration::hours(9)
}

async fn jjamdb_path(mode: &str) -> (Option<String>, Option<String>) {
    if let Ok(entries) = fs::read_dir("data/") {
        let file_names: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .collect();

        let mut latest_date: Option<NaiveDate> = None;
        let mut second_date: Option<NaiveDate> = None;
        for fname in file_names {
            if let Some(name_str) = fname.to_str() {
                if name_str.starts_with("jjam-") && name_str.ends_with(".sqlite") {
                    let date_str = &name_str[5..15];

                    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        if let Some(current_date) = latest_date.as_ref() {
                            if date > *current_date {
                                second_date = latest_date;
                                latest_date = Some(date);
                            }
                        } else {
                            latest_date = Some(date);
                        }
                    }
                }
            }
        }
        if let Some(date1) = latest_date {
            if let Some(date2) = second_date {
                return (
                    Some(format!("sqlite://data/jjam-{}.sqlite?mode={}", date1.format("%Y-%m-%d"), mode)),
                    Some(format!("sqlite://data/jjam-{}.sqlite?mode={}", date2.format("%Y-%m-%d"), mode))
                );
            }
            return (
                Some(format!("sqlite://data/jjam-{}.sqlite?mode={}", date1.format("%Y-%m-%d"), mode)),
                None
        );
        }
    }
    (None, None)
}

async fn get_jjam(date: chrono::NaiveDate) -> Result<Vec<JjamRow>, ShowError> {
    let dburi = jjamdb_path("ro").await;
    let mut ret: Vec<JjamRow> = Vec::new();
    
    if let Some(first) = dburi.0 {
        let db = SqlitePool::connect(&first).await?;
        let rows = sqlx::query_as::<_, JjamRow>("SELECT * FROM jjam WHERE dates=?")
            .bind(date.format("%Y-%m-%d").to_string()).fetch_all(&db).await?;
        
        if rows.len() > 0 {
            // 결과가 있으면 처리하고 반환
            for meal in rows {
                ret.push(meal);
            }
            return Ok(ret);
        }
    }
    
    if let Some(second) = dburi.1 {
        let db = SqlitePool::connect(&second).await?;
        let rows = sqlx::query_as::<_, JjamRow>("SELECT * FROM jjam WHERE dates=?")
            .bind(date.format("%Y-%m-%d").to_string()).fetch_all(&db).await?;
        
        for meal in rows {
            ret.push(meal);
        }
    }
    
    Ok(ret)
}


async fn get_menus(jjams: &Vec<JjamRow>, word: &str, meal: MealType) -> Result<String, ShowError> {
    let mut menus = format!("<b>{}</b> [{}kcal]\n", word, jjams[0].sum_cal);
    for r in jjams {
        let (menu, calorie) = meal.get_data(r);
        if menu.is_empty() && calorie.is_empty() {
            continue;
        }
        menus = format!("{}\n{} [{}kcal]", menus, menu, calorie);
    }
    Ok(menus)
}

async fn add_book(id: i64, timewhen: &str, booktype: &str, db: &sqlx::Pool<Sqlite>) -> Result<(), ShowError> {
    sqlx::query("INSERT OR REPLACE INTO timer (userid, timewhen, what) VALUES (?, ?, ?);")
        .bind(id)
        .bind(timewhen)
        .bind(booktype)
        .execute(db).await?;
    Ok(())
}

async fn del_book(id: i64, booktype: &str, db: &sqlx::Pool<Sqlite>) -> Result<(), ShowError> {
    sqlx::query("DELETE FROM timer WHERE userid=? AND what=?;")
        .bind(id)
        .bind(booktype)
        .execute(db).await?;
    Ok(())
}

async fn tg_reply_daemon() -> Result<(), ShowError> {
    let bot = Bot::from_env();
    if !Sqlite::database_exists("sqlite://data/users.sqlite?mode=ro").await.unwrap_or(false) {
        match Sqlite::create_database("sqlite://data/users.sqlite").await {
            Ok(_) => {
                println!("Database created");
                let db = SqlitePool::connect("sqlite://data/users.sqlite?mode=rw").await?;
                db.execute("PRAGMA foreign_keys=OFF;").await?;
                db.execute(r#"CREATE TABLE users(
                    id INTEGER PRIMARY KEY,
                    userid INTEGER not null UNIQUE,
                    username INTEGER DEFAULT null,
                    realname INTEGER DEFAULT null,
                    admin INTEGER NOT NULL CHECK(admin IN (0, 1)) DEFAULT 0
                );"#).await?;

                db.execute(r#"CREATE TABLE timer(
                id INTEGER PRIMARY KEY,
                userid INTEGER not null,
                timewhen TEXT not null,
                what TEXT CHECK(what IN ('breakfast', 'breakfastorder', 'nextbreakfast', 'nextbreakfastorder', 'lunch', 'lunchorder', 'dinner', 'dinnerorder', 'brunch', 'brunchorder', 'sundaybreakfast', 'sundaybreakfastorder')),
                UNIQUE (userid, what),
                FOREIGN KEY (userid) REFERENCES users(userid)
                );"#).await?;
            },
            Err(e) => {
                eprintln!("Error creating database: {}", e);
                return Ok(());
            }
        }
    }
    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        let db = SqlitePool::connect("sqlite://data/users.sqlite?mode=rw").await.map_err(|e| ShowError::from(e).tel_err())?;
        let words: Vec<&str> = msg.text().unwrap_or("").split_whitespace().collect();
        'done:{
            match words[0] {
                "/start" => {
                    bot.send_message(msg.chat.id, "환영합니다! help로 도움말을 확인하세요!").await?;
                    let _ = sqlx::query(
                        if words.len() == 3 && words[1] == "admin" && words[2] == "true" {
                            "INSERT INTO users (userid, username, realname, admin) VALUES (?, ?, ?, 1)"
                        } else {
                            "INSERT INTO users (userid, username, realname) VALUES (?, ?, ?)"
                        }
                    )
                        .bind(msg.chat.id.0)
                        .bind(msg.chat.username().unwrap_or(""))
                        .bind(format!("{} {}", msg.chat.first_name().unwrap_or(""), msg.chat.last_name().unwrap_or("")))
                        .execute(&db).await;
                }
                "help" => {
                    bot.send_message(msg.chat.id, r#"Help Message"#).await?;
                },
                "order"|"식사순서"|"식집순서" => {
                    let mut floor = 1;
                    let now = time_now();
                    if now.weekday() as u32 <= chrono::Weekday::Fri as u32 && (now.time() > NaiveTime::from_hms_opt(8, 30, 0).unwrap() && now.time() < NaiveTime::from_hms_opt(12, 0, 0).unwrap()) {
                        floor = 0;
                    }

                    bot.send_message(msg.chat.id, format!("<b>식집순서</b>: {}", ORDERS[(*ORDERIDX.lock().unwrap())[floor]]))
                            .parse_mode(ParseMode::Html).await?;
                },
                "아침"|"아침메뉴"|"아침식사" => {
                    bot.send_message(msg.chat.id, get_menus(
                        &get_jjam(time_now().date_naive()).await.unwrap_or_else(|e| {
                            eprintln!("daemon_error: {e}");
                            Vec::new()
                        }),
                        words[0],
                        MealType::Breakfast
                    ).await.map_err(|e| e.tel_err())?).parse_mode(ParseMode::Html).await?;
                },  
                "점심"|"점심메뉴"|"점심식사" => {
                    bot.send_message(msg.chat.id, get_menus(&get_jjam(time_now().date_naive()).await.unwrap_or_else(|e| {
                            eprintln!("daemon_error: {e}");
                            Vec::new()
                        }),
                        words[0],
                        MealType::Lunch
                    ).await.map_err(|e| e.tel_err())?).parse_mode(ParseMode::Html).await?;
                },
                "저녁"|"저녁메뉴"|"저녁식사" => {
                    bot.send_message(msg.chat.id, get_menus(&get_jjam(time_now().date_naive()).await.unwrap_or_else(|e| {
                            eprintln!("daemon_error: {e}");
                            Vec::new()
                        }),
                        words[0],
                        MealType::Dinner
                    ).await.map_err(|e| e.tel_err())?).parse_mode(ParseMode::Html).await?;
                },
                "낼아침"|"내일아침"|"내일아침메뉴"|"내일아침식사" => {
                    bot.send_message(msg.chat.id, get_menus(
                        &get_jjam((time_now() + chrono::Duration::days(1)).date_naive()).await.unwrap_or_else(|e| {
                            eprintln!("daemon_error: {e}");
                            Vec::new()
                        }),
                        words[0],
                        MealType::Breakfast
                    ).await.map_err(|e| e.tel_err())?).parse_mode(ParseMode::Html).await?;
                },
                "reserve"|"예약"|"등록" => {
                    'errorjmp:{
                        if words.len() < 2 {
                            break 'errorjmp;
                        }
                        let when = if words.len() >= 3 {
                            if let Ok(t) = NaiveTime::parse_from_str(&(words[2])[0..5], "%H:%M"){
                                t.with_second(0)
                            } else {
                                break 'errorjmp;
                            }
                        } else {
                            None
                        };
                        match words[1] {
                            "아침메뉴"|"아침식사" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_07_15.unwrap()).format("%H:%M").to_string(), "breakfastorder", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "아침식집"|"아침식집순서"|"아침식사순서" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_07_15.unwrap()).format("%H:%M").to_string(), "breakfast", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "점심메뉴"|"점심식사" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_11_00.unwrap()).format("%H:%M").to_string(), "lunch", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "점심식집"|"점심식집순서"|"점심식사순서" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_11_00.unwrap()).format("%H:%M").to_string(), "lunchorder", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "저녁메뉴"|"저녁식사" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_17_00.unwrap()).format("%H:%M").to_string(), "dinner", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "저녁식집"|"저녁식집순서"|"저녁식사순서" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_17_00.unwrap()).format("%H:%M").to_string(), "dinnerorder", &db).await.map_err(|e| e.tel_err())?;
                            },
                            "익일아침메뉴"|"익일아침식사" => {
                                add_book(msg.chat.id.0, &when.unwrap_or(HM_20_00.unwrap()).format("%H:%M").to_string(), "nextbreakfast", &db).await.map_err(|e| e.tel_err())?;
                            },
                            &_ => break 'errorjmp,
                        }
                        let _ = bot.send_message(msg.chat.id, if words.len() == 2 {
                            format!("{} 예약이 완료되었습니다.", words[1])
                        } else {
                            format!("{}에 {} 예약이 완료되었습니다.", words[2], words[1])
                        }).await;
                        break 'done;
                    }
                    let _ = bot.send_message(msg.chat.id, r#"
사용법은 다음과 같습니다.
예약 <종류> <시간(생략가능)>
예약 점심메뉴 11:00
<종류> 목록: 아침메뉴, 점심메뉴, 저녁메뉴, 익일아침메뉴
<시간> 입력시 시:분 형태로 입력해주십시오.
                    "#).await;

                },
               "delete"|"삭제"|"제거" => {
                    'errorjmp:{
                        if words.len() < 2 {
                            break 'errorjmp;
                        }
                        let success = match words[1] {
                            "아침메뉴"|"아침식사" => del_book(msg.chat.id.0, "breakfast", &db).await.is_ok(),
                            "아침식집"|"아침식집순서"|"아침식사순서" => del_book(msg.chat.id.0, "breakfastorder", &db).await.is_ok(),
                            "점심메뉴"|"점심식사" => del_book(msg.chat.id.0, "lunch", &db).await.is_ok(),
                            "점심식집"|"점심식집순서"|"점심식사순서" => del_book(msg.chat.id.0, "lunchorder", &db).await.is_ok(),
                            "저녁메뉴"|"저녁식사" => del_book(msg.chat.id.0, "dinner", &db).await.is_ok(),
                            "저녁식집"|"저녁식집순서"|"저녁식사순서" => del_book(msg.chat.id.0, "dinnerorder", &db).await.is_ok(),
                            "익일아침메뉴"|"익일아침식사" => del_book(msg.chat.id.0, "nextbreakfast", &db).await.is_ok(),
                            &_ => break 'errorjmp,
                        };
                        if !success {
                            break 'errorjmp;
                        }
                        let _ = bot.send_message(msg.chat.id, format!("{} 예약이 취소되었습니다.", words[1])).await;
                        break 'done;
                    }
                    let _ = bot.send_message(msg.chat.id, r#"
사용법은 다음과 같습니다.
예약 <종류> <시간(생략가능)>
예약 점심메뉴
<종류> 목록: 아침메뉴, 점심메뉴, 저녁메뉴, 익일아침메뉴
                    "#).await;

                },
                "admin"|"관리"|"설정" => {
                    if let Ok(v) = sqlx::query("SELECT 1 from users WHERE userid=? AND admin=1;")
                      .bind(msg.chat.id.0)
                      .fetch_one(&db)
                      .await {
                        {
                            let exists: u8 = v.get(0);
                            if exists != 1 {
                                break 'done;
                            }
                        }
                        if words.len() < 2 {
                            let _ = bot.send_message(msg.chat.id, r#"ERROR
                            "#).await;
                            break 'done;
                        }
                        let mut reply: String = String::new();
                        'adm_done:{
                            'adm_error:{
                                match words[1] {
                                    "changeorder"|"식사순서변경"|"식집순서변경"|"식사순서"|"식집순서" => {
                                        if words.len() < 4 {
                                            reply = format!("This needs 2 more argumemts (floor, changes)");
                                            break 'adm_error
                                        }
                                        let ordr = update_orderidx(match words[2].parse::<u32>() {
                                            Ok(floor) => (floor - 1) as usize,
                                            _ => break 'adm_error,
                                        },
                                        match words[3].parse::<i32>() {
                                            Ok(v) => v,
                                            _ => break 'adm_error,
                                        });
                                        reply = format!("Now {}", ORDERS[ordr]);
                                        break 'adm_done;
                                    },
                                    _ => {},
                                }
                            }
                            let idx = [update_orderidx(0, 0), update_orderidx(1, 0)];
                            let orders = [ORDERS[idx[0]], ORDERS[idx[1]]];
                            let _ = bot.send_message(msg.chat.id, format!("ERROR: {} \n사용법: {} {} <층수> <차이>\n1층: {}\n2층: {}", reply, words[0], words[1], orders[0], orders[1])).await;
                            break 'done;
                        }
                        let _ = bot.send_message(msg.chat.id, format!("{}", reply)).await;
                        break 'done;
                    }
                },
                "time"|"date"|"datetime"|"시간" => {
                    let _ = bot.send_message(msg.chat.id, (time_now()).format("%Y-%m-%d %H:%M:%S").to_string()).await;
                },

                &_ => {},
            }
        }
        println!("{}", msg.chat.id);
        Ok(())
    }).await;
    Ok(())
}

async fn get_last_order(floor: usize, now: NaiveDateTime, db: &Pool<Sqlite>) -> Result<u32, ShowError> {
    match sqlx::query("SELECT orderidx FROM orders WHERE floor=? ORDER BY datestime DESC LIMIT 1;")
    .bind(floor as u32)
    .fetch_one(db).await {
        Ok(row) => {
            Ok(row.get(0))
        },
        Err(e) => {
            eprintln!("Warning: {}\n order: {}", e, ORDERS[0]);
            sqlx::query("INSERT INTO orders (datestime, floor, ordertxt, orderidx) VALUES (?, ?, ?, ?);")
                .bind(now.format("%Y-%m-%d %H:%M").to_string())
                .bind(0)
                .bind(ORDERS[0])
                .bind(0)
                .execute(db)
                .await?;

            Ok(0)
        }
    }
}

async fn insert_order(floor: usize, now: NaiveDateTime, db: &Pool<Sqlite>) -> Result<(), ShowError> {
    let v = update_orderidx(floor, 1);
    sqlx::query("INSERT INTO orders (datestime, floor, ordertxt, orderidx) VALUES (?, ?, ?, ?);")
        .bind(now.format("%Y-%m-%d %H:%M").to_string())
        .bind(floor as u32)
        .bind(ORDERS[v])
        .bind(v as u32)
        .execute(db)
        .await?;
    Ok(())
}

async fn jjam_alarm() -> Result<(), ShowError>{
    let mut now = time_now();
    let orderdb;
    
    println!("{:?}", update_orderidx(0, 0));
    { // Init Orders DB
        if !Sqlite::database_exists("sqlite://data/orders.sqlite?mode=ro").await.unwrap_or(false) {
            match Sqlite::create_database("sqlite://data/orders.sqlite").await {
                Ok(_) => {
                    println!("Order Log Database created");
                    orderdb = SqlitePool::connect("sqlite://data/orders.sqlite?mode=rw").await?;
                    orderdb.execute(r#"CREATE TABLE orders(
                        id INTEGER PRIMARY KEY,
                        datestime TEXT,
                        floor INTEGER CHECK(orderidx IN (0, 1)),
                        ordertxt TEXT,
                        orderidx INTEGER CHECK(orderidx IN (0, 1, 2))
                    );"#).await?;
                },
                Err(e) => {
                    panic!("Setup orders.sqlite failed: {}", e);
                }
            }
        } else {
            orderdb = SqlitePool::connect("sqlite://data/orders.sqlite?mode=rw").await?;
        }

        let mut ordridx = ORDERIDX.lock().unwrap();
        (*ordridx)[0] = get_last_order(0, now.naive_utc(), &orderdb).await? as usize;
        (*ordridx)[1] = get_last_order(1, now.naive_utc(), &orderdb).await? as usize;
        drop(ordridx);
    }

    let client = reqwest::Client::new();
    let tg_token = std::env::var(TG_TOKEN).unwrap();
    let mut jjams = get_jjam(time_now().date_naive()).await.unwrap_or_else(|e| {
        eprintln!("daemon_error: {e}");
        Vec::new()
    });
    let timerdb = SqlitePool::connect("sqlite://data/users.sqlite?mode=ro").await?;
    loop {
        now = time_now().with_second(0).unwrap().with_nanosecond(0).unwrap();

        match Some(now.time()) {
            HM_00_00 => {
                jjams = get_jjam(now.date_naive()).await.unwrap_or_else(|e| {
                    eprintln!("daemon_error: {e}");
                    Vec::new()
                });
            },
            HM_08_00 => if now.weekday() != chrono::Weekday::Sat {
                insert_order(1, now.naive_utc(), &orderdb).await?;
            },
            HM_11_00 => if now.weekday() == chrono::Weekday::Sat {
                insert_order(1, now.naive_utc(), &orderdb).await?;
            },
            HM_12_00 => if now.weekday() as u32 <= chrono::Weekday::Fri as u32 {
                insert_order(0, now.naive_utc(), &orderdb).await?;
            } else if now.weekday() == chrono::Weekday::Sun {
                insert_order(1, now.naive_utc(), &orderdb).await?;
            },
            HM_18_00 => {
                insert_order(1, now.naive_utc(), &orderdb).await?
            },
            _ => {},
        }

        let rows = sqlx::query("SELECT * FROM timer WHERE timewhen=?")
            .bind(now.format("%H:%M").to_string()).fetch_all(&timerdb).await?;

        for r in rows {
            let userid = r.get::<i64, &str>("userid");
            let msgtype = r.get::<&str, &str>("what");
            let msg = match msgtype {
                "breakfast" => get_menus(&jjams, "아침 메뉴", MealType::Breakfast).await?,
                "breakfastorder" => format!("식사순서: {}", ORDERS[(*ORDERIDX.lock().unwrap())[1]]),
                "lunch" => if now.weekday() == chrono::Weekday::Sat { continue; } else {
                    get_menus(&jjams, "점심 메뉴", MealType::Lunch).await?
                },
                "lunchorder" => if now.weekday() == chrono::Weekday::Sat { continue; } else {
                    format!("식사순서: {}", ORDERS[(*ORDERIDX.lock().unwrap())[
                        if now.weekday() as u32 > chrono::Weekday::Fri as u32 {
                            1
                        } else {
                            0
                        }
                    ]])
                },
                "dinner" => get_menus(&jjams, "저녁 메뉴", MealType::Dinner).await?,
                "dinnerorder" => format!("식사순서: {}", ORDERS[(*ORDERIDX.lock().unwrap())[1]]),
                "nextbreakfast" => get_menus(&jjams, "내일 아침 메뉴", MealType::Breakfast).await?,
                "nextbreakfastorder" => format!("식사순서: {}", ORDERS[(*ORDERIDX.lock().unwrap())[1]]),
                _ => continue,
            };

            let _ = client
                .get(format!("https://api.telegram.org/bot{}/sendMessage?chat_id={}&parse_mode=HTML&text={}", tg_token, userid, msg))
                .header(reqwest::header::USER_AGENT, USER_AGENT_FIREFOX)
                .send()
            .await?;
        }
        let duration_time = (time_now().duration_trunc(Duration::minutes(1)).unwrap() + Duration::minutes(1)).signed_duration_since(time_now());
        assert!(duration_time > Duration::zero(), "duration time is {duration_time}");
        tokio::time::sleep(duration_time.to_std().unwrap()).await;
    }
}

async fn jjam_poll() -> Result<(), ShowError> {
    let mut jjam_count: i32;
    let mut jjam_dbcnt: i32 = 0;
    let mut dburi: (Option<String>, Option<String>) = jjamdb_path("rw").await;
    let mut db: sqlx::Pool<Sqlite>;
    let jjam_token = std::env::var(JJAM_TOKEN).unwrap();
    let unit_code = std::env::var(UNIT_CODE).unwrap();

    // 식단 영역과 칼로리 영역 관리가 엉망이라, 일부 경우에 대해서 수동으로 위치를 서로 바꿈
    // 예 1: (meal="320kcal", calorie="밤양갱") -> ("밤양갱", "320")
    // 예 2: (meal="", calorie="밤양갱") -> ("밤양갱", "")
    let process_calorie = |meal: &str, calorie: &str| -> (String, String) {
        if !calorie.ends_with("kcal") {
            if meal.ends_with("kcal") {
                (calorie.to_string(), (&meal[0..meal.len() - 4]).to_string())
            } else {
                (calorie.to_string(), String::new())
        }
        } else {
            (meal.to_string(), (&calorie[0 .. calorie.len() - 4]).to_string())
        }
    };


    let client = reqwest::Client::new();
    loop{
        let res = client
            .get(format!("https://openapi.mnd.go.kr/{}/json/DS_TB_MNDT_DATEBYMLSVC_{}/1/1", jjam_token, unit_code))
            .header(reqwest::header::USER_AGENT, USER_AGENT_FIREFOX)
            .send()
            .await?;

            jjam_count = serde_json::from_str::<JjamDataShell>(&res.text().await?)
                .map_err(|e| e.to_string())?
                .DS_TB_MNDT_DATEBYMLSVC_7021.list_total_count;

        'db_init:{
            if let Some(first) = &dburi.0 {
                println!("DB PATH: {}", first);
                db = SqlitePool::connect(&first).await?;
                let row = sqlx::query("SELECT COUNT(*) FROM jjam")
                    .fetch_one(&db)
                    .await?;
                jjam_dbcnt = row.get(0);
                println!("{}", jjam_dbcnt);
            }

            if jjam_count != jjam_dbcnt {
                dburi.1 = dburi.0.clone();
                dburi.0 = Some(format!("sqlite://data/jjam-{}.sqlite?mode=rw", time_now().date_naive()));
                if !Sqlite::database_exists(&dburi.0.clone().unwrap()).await.unwrap_or(false) {
                    match Sqlite::create_database(&dburi.0.clone().unwrap()).await {
                        Ok(_) => {
                            println!("Database created");
                            db = SqlitePool::connect(&dburi.0.clone().unwrap()).await?;
                            db.execute(r#"CREATE TABLE jjam(
                                id INTEGER PRIMARY KEY,
                                dates TEXT,
                                brst TEXT,
                                brst_cal TEXT,
                                lunc TEXT,
                                lunc_cal TEXT,
                                dinr TEXT,
                                dinr_cal TEXT,
                                adspcfd TEXT,
                                adspcfd_cal TEXT,
                                sum_cal TEXT
                            );"#).await?;
                        },
                        Err(e) => {
                            eprintln!("Error creating database: {}", e);
                            return Ok(());
                        },
                    }
                } else {
                    println!("Database already exists");
                    break 'db_init;
                }
                db.execute("PRAGMA journal_mode=WAL").await?;

                let json = client
                .get(format!("https://openapi.mnd.go.kr/{}/json/DS_TB_MNDT_DATEBYMLSVC_{}/1/{}", jjam_token, unit_code, jjam_count))
                .header(reqwest::header::USER_AGENT, USER_AGENT_FIREFOX)
                .send()
                .await?
                .text()
                .await?;

                let jjam;

                {
                    let jjamshell: JjamDataShell = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                    jjam = jjamshell.DS_TB_MNDT_DATEBYMLSVC_7021;
                }
                let row = jjam.row.borrow();

                for meal in &mut row.iter() {
                    let mut meal = meal.clone();
                    println!("{:?}", meal);
                    meal.dates = if meal.dates.len() < 10 {
                        meal.dates.clone()
                    } else {
                        (&meal.dates.clone()[0..10]).to_string()
                    };
                    (meal.brst, meal.brst_cal) = process_calorie(&meal.brst, &meal.brst_cal);
                    (meal.lunc, meal.lunc_cal) = process_calorie(&meal.lunc, &meal.lunc_cal);
                    (meal.dinr, meal.dinr_cal) = process_calorie(&meal.dinr, &meal.dinr_cal);
                    (meal.adspcfd, meal.adspcfd_cal) = process_calorie(&meal.adspcfd, &meal.adspcfd_cal);
                    meal.sum_cal = if meal.sum_cal.len() < 4 {
                        meal.sum_cal.clone()
                    } else {
                            (&meal.sum_cal.clone()[0 .. meal.sum_cal.len() - 4]).to_string()
                    };


                    sqlx::query("INSERT INTO jjam (dates, brst, brst_cal, lunc, lunc_cal, dinr, dinr_cal, adspcfd, adspcfd_cal, sum_cal) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                        .bind(&meal.dates)
                        .bind(&meal.brst)
                        .bind(&meal.brst_cal)
                        .bind(&meal.lunc)
                        .bind(&meal.lunc_cal)
                        .bind(&meal.dinr)
                        .bind(&meal.dinr_cal)
                        .bind(&meal.adspcfd)
                        .bind(&meal.adspcfd_cal)
                        .bind(&meal.sum_cal)
                        .execute(&db).await?;
                }
            }
        }
        let local_time = chrono::Local::now();
        let redo_time = (chrono::Local::now() + chrono::Duration::days(1))
            .with_hour(3).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap();

        if local_time < redo_time {
            let duration_time = (redo_time - local_time) + chrono::Duration::hours(3);
            sleep(duration_time.to_std().unwrap()).await;
        }
    }
}

#[tokio::main]
async fn main() {
    tokio::select! {
        poll_result = jjam_poll() => {
            if let Err(e) = poll_result {
                eprintln!("poll_error: {e}");
            }
        },
        alarm_result = jjam_alarm() => {
            if let Err(e) = alarm_result {
                eprintln!("alarm_error: {e}");
            }
        },
        daemon_result = tg_reply_daemon() => {
            if let Err(e) = daemon_result {
                eprintln!("daemon_error: {e}");
            }
        },
    }
}

fn update_orderidx(floor: usize, change: i32) -> usize {
    let mut v = ORDERIDX.lock().unwrap();
    let floor = floor & 1;
    (*v)[floor] += change as usize;
    (*v)[floor] %= 3;
    let ret = (*v)[floor];

    return ret;
}
