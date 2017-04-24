extern crate ncurses;
extern crate nix;

use std::fmt::Display;
use std::fmt::Formatter;

use std::path::Path;
use std::path::PathBuf;
use std::fs::read_dir;
use std::fs::canonicalize;

use std::char;
use ncurses::*;

use nix::sys::signal::SigAction;
use nix::sys::signal::SigHandler;
use nix::sys::signal::SaFlags;
use nix::sys::signal::SigSet;
use nix::sys::signal::sigaction;

use nix::sys::signal::SIGINT;

#[allow(dead_code)]
enum Direction {
    Horizontal,
    Vertical,
}

impl Display for Direction {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            &Direction::Horizontal => write!(f, "Hor"),
            &Direction::Vertical   => write!(f, "Ver"),
        }
    }
}

#[allow(dead_code)]
enum Operation {
    Open(usize),
    Save(usize),
    Merge(usize, usize, Direction),
    Crop(usize, u32, u32, i32, i32),
}

const KEY_Q         : i32 = 'q' as i32;
const KEY_O         : i32 = 'o' as i32;
const KEY_TAB       : i32 = 0x09;
const KEY_ENTER     : i32 = 0x0a;
const KEY_BACKSPACE : i32 = 0x7f;
const KEY_ESC       : i32 = 0x1b;
const KEY_DOWN      : i32 = 0x102;
const KEY_UP        : i32 = 0x103;
const KEY_LEFT      : i32 = 0x104;
const KEY_RIGHT     : i32 = 0x105;

extern fn stop_program(_: i32) {
    endwin();
    std::process::exit(0);
}

const ERROR_COLOR     : i16 = 1;
const HIGHLIGHT_COLOR : i16 = 2;

// Reference:
// https://github.com/jeaye/ncurses-rs/blob/master/src/ncurses.rs
fn main() {

    /* Installing a SIGINT handler */
    let mut sig_set = SigSet::empty();
    sig_set.add(SIGINT);
    let sig_action = SigAction::new(SigHandler::Handler(stop_program), SaFlags::all(), sig_set);
    #[allow(unused_must_use)]
    unsafe { sigaction(SIGINT, &sig_action); }

    /* Start ncurses. */
    initscr();
    raw();
    start_color();
    keypad(stdscr(), true);
    noecho();

    init_pair(ERROR_COLOR, COLOR_WHITE, COLOR_RED);
    init_pair(HIGHLIGHT_COLOR, COLOR_WHITE, COLOR_BLACK);

    let screen_width  = getmaxx(stdscr());
    let mut screen_height = getmaxy(stdscr());

    // NOTE(erick): Creating space for the minibuffer.
    wresize(stdscr(), screen_height - 1, screen_width);
    screen_height -= 1;

    let minibuffer_window = newwin(1, screen_width, screen_height, 0);

    let opened_files_window_width = screen_width / 2;
    let opened_files_window = newwin(screen_height, opened_files_window_width, 0, 0);

    let operations_window_width = screen_width - opened_files_window_width;
    let operations_window = newwin(screen_height, operations_window_width,
                                   0, opened_files_window_width);

    let mut selected_operation = -1;

    // NOTE(erick): The name 'opened_files' is a bit misleading
    // since the files are only opened when the manipulation is
    // applied.
    let mut opened_files = Vec::new();
    let mut operations = Vec::new();
    loop {
        // wprint_strings(stdscr(), &opened_files);
        clear_window(minibuffer_window);
        clear_window(operations_window);
        clear_window(opened_files_window);

        wprint_files(opened_files_window, &opened_files);
        wrefresh(opened_files_window);

        wprint_operations(operations_window,
                          &operations, &opened_files,
                          selected_operation);
        wrefresh(operations_window);

        refresh();

        let mut key_o_pressed = false;

        let ch = getch();
        match ch {
            KEY_Q => { break; },
            KEY_O => { key_o_pressed = true },

            _     => { },
        };

        if key_o_pressed {
            let new_file = open_file(minibuffer_window, screen_height, screen_width);
            if new_file.is_some() {
                opened_files.push(new_file.unwrap());
                operations.push(Operation::Open(opened_files.len() - 1));
            }
        }
    }

    endwin();
}

#[allow(unused_variables, unused_assignments)]
fn open_file(win: WINDOW,screen_height: i32, screen_width: i32) -> Option<PathBuf> {
    let mut string = get_current_path();

    let mut done = false;
    let mut do_open_file = false;
    let mut is_autocompleting = false;
    let mut completion = None;
    let mut completion_index = 0;

    loop {
        wclear(win);
        wmove(win, 0, 0);
        wprintw(win, "File: ");
        wprintw(win, string.as_ref());
        wrefresh(win);

        wbkgd(win, COLOR_PAIR(0));

        let mut auto_complete = false;
        let mut restart_state = true;

        let ch = getch();
        match ch {
            KEY_ENTER     => { done = true; do_open_file = true; },
            KEY_ESC       => { done = true; do_open_file = false; },
            KEY_TAB       => { auto_complete = true; restart_state = false; },
            KEY_BACKSPACE => { string.pop(); },
            _             => { string.push(get_char(ch)); },
        };

        if auto_complete {
            if !is_autocompleting {
                is_autocompleting = true;
                completion = get_maximum_path_matching(string.as_ref());
                if completion.is_some() {
                    let completion = completion.as_ref().unwrap();
                    if completion.len() > 0 {
                        string = maximum_prefix(completion);
                    }  else {
                        wbkgd(win, COLOR_PAIR(ERROR_COLOR));
                    }
                }
            } else {
                if completion.is_some() {
                    let options = completion.as_ref().unwrap();
                    if options.len() > 0 {
                        string = options[completion_index].clone();
                        completion_index += 1;
                        if completion_index >= options.len() {
                            completion_index = 0;
                        }
                    } else {
                        wbkgd(win, COLOR_PAIR(ERROR_COLOR));
                    }
                } else {
                    wbkgd(win, COLOR_PAIR(ERROR_COLOR));
                }
            }
        }

        if done {
            if !do_open_file {
                return None;
            } else {
                if let Ok(path_buf) = handle_file_opening(&string) {
                    return Some(path_buf);
                } else {
                    wbkgd(win, COLOR_PAIR(ERROR_COLOR));
                    done = false;
                }
            }
        }

        if restart_state {
            is_autocompleting = false;
            completion_index = 0;
        }
    }
}

// NOTE(erick): Since we don't have a goto statement
// this function was extracted from the code above so
// we can do early-outs an keep the code more readable.
fn handle_file_opening(string: &String) -> Result<PathBuf, ()> {
    // TODO(erick): We already had a PathBuf before,
    // we should not have to construct one here.
    let path = Path::new(string.as_str());
    let meta_data = std::fs::metadata(path);

    if meta_data.is_err() {
        return Err ( () )
    }

    let meta_data = meta_data.unwrap();
    if !meta_data.is_file() {
        return Err( () )
    }

    let mut path_buf = PathBuf::new();
    path_buf.push(path);

    // NOTE(erick): Isolating the scope of
    // extension to make the compiler happy.
    {
        let extension = path_buf.extension();
        if extension.is_none() {
            return Err ( () )
        }

        let extension = extension.unwrap();
        if extension != "bmp" {
            return Err ( () )
        }
    }

    Ok(path_buf)
}

fn get_current_path() -> String {
    let path = canonicalize(".");
    if path.is_err() {
        return String::from("");
    }

    let path = path.unwrap();
    let result = path.into_os_string().into_string();
    if result.is_err() {
        return String::from("");
    }

    result.unwrap()
}

fn get_maximum_path_matching(to_complete: &str) -> Option<Vec<String> > {
    let last_slash_index = to_complete.rfind('/');
    if last_slash_index.is_none() {
        return None;
    }

    let last_slash_index = last_slash_index.unwrap();
    let path_to_search = &to_complete[0 .. last_slash_index];
    let string_to_match = &to_complete[last_slash_index + 1 ..];

    if string_to_match.len() == 0 {
        return None;
    }

    let dir_iterator = read_dir(path_to_search);
    if dir_iterator.is_err() {
        return None;
    }

    let mut matching_files = Vec::new();
    let dir_iterator = dir_iterator.unwrap();
    // NOTE(erick): We could do a fuzzy search or
    // try to correct the string using Levenshtein distance
    // but I will keep it simple for now.
    for dir in dir_iterator {
        if dir.is_err() {
            // TODO(erick): Log this?
            return None;
        }

        let dir = dir.unwrap();
        let path = dir.path();
        let filename = path.file_name();
        // NOTE(erick): paths like .. don't have a file_name.
        if filename.is_none() {
            continue;
        }

        let filename = filename.unwrap().to_string_lossy();
        if filename.starts_with(string_to_match) {
            matching_files.push(path.clone().
                                into_os_string().into_string().unwrap_or_default());
        }
    }

    Some(matching_files)
}

fn maximum_prefix(strings: &Vec<String>) -> String {
    if strings.len() == 0 {
        return "".to_string();
    }
    let mut strings_chars = Vec::new();

    for string in strings {
        strings_chars.push(string.chars().collect::<Vec<_> >());
    }

    let mut result_len = 0;
    'outter: loop {
        if result_len == strings_chars[0].len() {break 'outter; }

        let ch = strings_chars[0][result_len];
        for string_chars in &strings_chars {
            if result_len == string_chars.len() { break 'outter; }
            if string_chars[result_len] != ch { break 'outter; }
        }

        result_len += 1;
    }

    let mut result = String::new();
    for i in 0 .. result_len {
        result.push(strings_chars[0][i]);
    }

    result
}

fn clear_window(win: WINDOW) {
    wbkgd(win, COLOR_PAIR(0));
    wclear(win);
    wrefresh(win);
}

fn get_char(ch: i32) -> char {
    char::from_u32(ch as u32).expect("Invalid char")
}

#[allow(dead_code)]
fn wprint_i32_char(win: WINDOW, ch: i32) {
    wprintw(win, format!("{}", get_char(ch)).as_ref());
}

#[allow(dead_code)]
fn wprint_i32_char_debug(win: WINDOW, ch: i32) {
    wprintw(win, format!("{}: {:08x}", get_char(ch), ch).as_ref());
}

#[allow(dead_code)]
fn wprint_strings(win: WINDOW, strings: &Vec<String>) {
    let mut line_numer = 0;
    for string in strings {
        wmove(win, line_numer, 0);
        wprintw(win, string.as_ref());

        line_numer += 1;
    }
}

fn wprint_files(window: WINDOW, files: &Vec<PathBuf>) {
    wmove(window, 0, 0);
    wprintw(window, "Opened files:");

    let mut file_number = 1;
    for file in files {
        wmove(window, file_number, 0);

        wprintw(window, format!("{}: {}",
                                file_number, file_stem(file)).as_str());

        file_number += 1;
    }
}

#[allow(unused_variables)]
fn wprint_operations(window: WINDOW,
                     operations: &Vec<Operation>, opened_files: &Vec<PathBuf>,
                     selected_operation: i32) {
    wmove(window, 0, 0);
    wprintw(window, "Operations:");

    // NOTE(erick): If selected_operation is -1 (meaning no operation is
    // selected) selected_number will be zero an no entry will be highlighted.
    let selected_number = selected_operation + 1;
    let mut operation_number = 1;

    for operation in operations {
        wmove(window, operation_number, 0);

        if operation_number == selected_number {
            wbkgd(window, COLOR_PAIR(HIGHLIGHT_COLOR));
        }

        wprintw(window, format!("{}: ", operation_number).as_str());

        match operation {
            &Operation::Open(file_index) => {
                wprintw(window, format!("Open({})",
                                        file_stem(&opened_files[file_index])).as_str());

            },
            &Operation::Save(file_index) => {
                wprintw(window, format!("Save({})",
                                        file_stem(&opened_files[file_index])).as_str());
            },
            &Operation::Merge(op1_index, op2_index, ref dir) => {
                wprintw(window, format!("Merge({}, {}, {})",
                                        op1_index, op2_index, dir).as_str());
            },
            &Operation::Crop(op_index, x, y, w, h) => {

            },
        }

        if operation_number == selected_number {
            wbkgd(window, COLOR_PAIR(0));
        }
        operation_number += 1;
    }
}

fn file_stem (path: &PathBuf) -> String {
    let file_stem = path.file_stem();
    if file_stem.is_none() {
        return "NO NAME".to_string();
    }

    let file_stem = file_stem.unwrap();
    let result = file_stem.to_os_string().into_string();
    if result.is_err() {
        return "NO NAME".to_string();
    }

    result.unwrap()
}
