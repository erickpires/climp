#[macro_use]
extern crate scopeguard;
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
use ncurses::CURSOR_VISIBILITY::CURSOR_INVISIBLE;

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
    Save(usize, usize),
    Merge(usize, usize, Direction),
    Crop(usize, u32, u32, i32, i32),
}

impl Display for Operation {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            &Operation::Open(ref file)
                => write!(f, "Open({})", file),
            &Operation::Save(ref op, ref file)
                => write!(f, "Save({}, {})", op, file),
            &Operation::Crop(ref op, ref x0, ref y0, ref w, ref h)
                => write!(f, "Crop({}, {}, {}, {}, {})", op, x0, y0, w, h),
            &Operation::Merge(ref op0, ref op1, ref dir)
                => write!(f, "Merge({}, {}, {})", op0, op1, dir),
        }
    }
}

const KEY_C         : i32 = 'c' as i32;
const KEY_M         : i32 = 'm' as i32;
const KEY_O         : i32 = 'o' as i32;
const KEY_Q         : i32 = 'q' as i32;
const KEY_S         : i32 = 's' as i32;
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

const NORMAL_COLOR    : i16 = 1;
const ERROR_COLOR     : i16 = 2;
const HIGHLIGHT_COLOR : i16 = 3;
const QUESTION_COLOR  : i16 = 4;

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
    init_pair(HIGHLIGHT_COLOR, COLOR_BLACK, COLOR_WHITE);
    init_pair(NORMAL_COLOR, COLOR_WHITE, COLOR_BLACK);
    init_pair(QUESTION_COLOR, COLOR_WHITE, COLOR_BLUE);

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
                          &operations, &opened_files, -1);
        wrefresh(operations_window);

        refresh();

        let mut key_o_pressed = false;
        let mut key_s_pressed = false;
        let mut key_m_pressed = false;
        let mut key_c_pressed = false;

        let ch = getch();
        match ch {
            KEY_Q => { break; },
            KEY_O => { key_o_pressed = true },
            KEY_S => { key_s_pressed = true },
            KEY_M => { key_m_pressed = true },
            KEY_C => { key_c_pressed = true },

            _     => { },
        };

        if key_o_pressed {
            let new_file = open_file(minibuffer_window,
                                     screen_height, screen_width, true);
            if new_file.is_some() {
                opened_files.push(new_file.unwrap());
                operations.push(Operation::Open(opened_files.len() - 1));
            }
        }

        if key_s_pressed {
            let new_file = open_file(minibuffer_window,
                                     screen_height, screen_width, false);
            if new_file.is_some() {
                opened_files.push(new_file.unwrap());
                operations.push(Operation::Save(0, opened_files.len() - 1));
            }
        }

        if key_m_pressed {
            let op = get_merge_operation(minibuffer_window, operations_window,
                                         &operations, &opened_files);
            if op.is_some() {
                operations.push(op.unwrap());
            }
        }

        if key_c_pressed {
            let op = get_crop_operation(minibuffer_window, operations_window,
                                        &operations, &opened_files);
            if op.is_some() {
                operations.push(op.unwrap());
            }
        }
    }

    endwin();
}

fn get_merge_operation(minibuffer_window: WINDOW, operations_window: WINDOW,
                       operations: &Vec<Operation>,
                       opened_files: &Vec<PathBuf>) -> Option<Operation> {
    let operation0 = select_operation(minibuffer_window, operations_window,
                                      &operations, &opened_files,
                                      "Merge: (");
    if operation0.is_none() { return None; }

    let operation0 = operation0.unwrap();
    let prompt = format!("Merge: ({}, ", operation0 + 1);
    let operation1 = select_operation(minibuffer_window, operations_window,
                                      &operations, &opened_files,
                                      prompt.as_str());

    if operation1.is_none() { return None; }

    let operation1 = operation1.unwrap();
    let direction = select_direction(minibuffer_window);

    if direction.is_none() { return None; }

    let direction = direction.unwrap();

    let confirmation_prompt = format!("Merge({}, {}, {})",
                                      operation0, operation1, direction);
    let confirmation = get_confirmation(minibuffer_window,
                                        confirmation_prompt.as_str());
    if !confirmation { return None; }

    Some(Operation::Merge(operation0, operation1, direction))
}

fn get_crop_operation(minibuffer_window: WINDOW, operations_window: WINDOW,
                       operations: &Vec<Operation>,
                       opened_files: &Vec<PathBuf>) -> Option<Operation> {
    let operation = select_operation(minibuffer_window, operations_window,
                                      &operations, &opened_files,
                                      "Merge: (");
    if operation.is_none() { return None; }

    let x0 = enter_u32(minibuffer_window, "X0: ");
    if x0.is_none() { return None; }

    let y0 = enter_u32(minibuffer_window, "Y0: ");
    if y0.is_none() { return None; }


    let width = enter_u32(minibuffer_window, "WIDTH: ");
    if width.is_none() { return None; }


    let height = enter_u32(minibuffer_window, "HEIGHT: ");
    if height.is_none() { return None; }


    let operation = operation.unwrap();
    let x0 = x0.unwrap();
    let y0 = y0.unwrap();
    let width = width.unwrap() as i32;
    let height = height.unwrap() as i32;

    let confirmation_prompt = format!("Crop({}, {}, {}, {}, {})",
                                      operation,
                                      x0, y0,
                                      width, height);
    let confirmation = get_confirmation(minibuffer_window,
                                        confirmation_prompt.as_str());
    if !confirmation { return None; }

    Some(Operation::Crop(operation, x0, y0, width, height))
}

fn get_confirmation(minibuffer: WINDOW, prompt: &str) -> bool {
    clear_window(minibuffer);
    change_to_color(minibuffer, QUESTION_COLOR);
    wprintw(minibuffer, prompt);
    wprintw(minibuffer, " : Confirm?");
    wrefresh(minibuffer);

    loop {
        let ch = getch();
        match ch {
            KEY_ENTER => { return true; },
            KEY_ESC   => { return false; },
            KEY_Q     => { return false; },
            _         => {  },
        }
    }
}

fn select_operation(minibuffer: WINDOW, window: WINDOW,
                    operations: &Vec<Operation>,
                    opened_files: &Vec<PathBuf>,
                    prompt: &str) -> Option<usize> {
    // NOTE(erick): Don't bother selecting from an empty list
    if operations.len() == 0 { return None; }

    let old_cursor = curs_set(CURSOR_INVISIBLE);
    defer! {
        if old_cursor.is_some() {
            curs_set(old_cursor.unwrap());
        }
    }

    clear_window(minibuffer);
    wprintw(minibuffer, prompt);
    wrefresh(minibuffer);

    let mut selected: isize = 0;
    loop {
        let mut selected_increment = 0;

        clear_window(window);
        wprint_operations(window,
                          operations, opened_files, selected as isize);
        wrefresh(window);

        let ch = getch();
        match ch {
            KEY_ENTER => { return Some(selected as usize); },
            KEY_ESC   => { return None; },
            KEY_Q     => { return None; },
            KEY_UP    => { selected_increment = -1; },
            KEY_DOWN  => { selected_increment =  1; },
            _         => { },
        }

        if selected_increment != 0 {
            selected += selected_increment;
            if selected < 0 {
                selected = (operations.len() - 1) as isize;
            }
            if selected as usize == operations.len() {
                selected = 0;
            }
        }
    }
}

fn select_direction(minibuffer: WINDOW) -> Option<Direction> {
    let options = vec!['H', 'V'];
    let chosen = select_from_options(minibuffer, &options, "Direction: ");

    if chosen.is_none() {
        return None;
    }

    match chosen.unwrap() {
        'H' => Some(Direction::Horizontal),
        'V' => Some(Direction::Vertical),
        _   => None,
    }
}

fn select_from_options(minibuffer: WINDOW,
                       options: &Vec<char>,
                       prompt: &str) -> Option<char> {
    if options.len() == 0 { return None; }

    let old_cursor = curs_set(CURSOR_INVISIBLE);
    defer! {
        if old_cursor.is_some() {
            curs_set(old_cursor.unwrap());
        }
    }

    let mut selected = 0;
    loop {
        clear_window(minibuffer);
        wprintw(minibuffer, prompt);

        let mut option_index = 0;
        for c in options {
            if option_index == selected {
                change_to_color(minibuffer, HIGHLIGHT_COLOR);
                waddch(minibuffer, *c as u32);
                change_to_color(minibuffer, NORMAL_COLOR);
            } else {
                waddch(minibuffer, *c as u32);
            }
            waddch(minibuffer, ' ' as u32);

            option_index += 1;
        }

        wrefresh(minibuffer);

        let mut selected_increment = 0;

        let ch = getch();
        match ch {
            KEY_ENTER => { return Some(options[selected as usize]); },
            KEY_ESC   => { return None; },
            KEY_Q     => { return None; },
            KEY_LEFT  => { selected_increment = -1; },
            KEY_RIGHT => { selected_increment =  1; },
            _         => { },
        }

        if selected_increment != 0 {
            selected += selected_increment;
            if selected < 0 {
                selected = (options.len() - 1) as isize;
            }
            if selected as usize == options.len() {
                selected = 0;
            }
        }
    }
}

fn enter_u32(minibuffer: WINDOW, prompt: &str) -> Option<u32> {
    let mut string = String::new();
    loop {
        wclear(minibuffer);
        wmove(minibuffer, 0, 0);
        wprintw(minibuffer, prompt);
        wprintw(minibuffer, string.as_str());
        wrefresh(minibuffer);

        change_to_color(minibuffer, NORMAL_COLOR);

        let mut char_to_push = None;
        let mut done = false;

        let ch = getch();
        match ch {
            KEY_ENTER     => { done = true; },
            KEY_ESC       => { return None; },
            KEY_Q         => { return None; },
            KEY_BACKSPACE => { string.pop(); },
            _             => { char_to_push = Some(ch) },
        };

        if char_to_push.is_some() {
            let char_to_push = get_char(char_to_push.unwrap());
            match char_to_push {
                ch @ '0' ... '9' => { string.push(ch); },
                _               => { change_to_color(minibuffer, ERROR_COLOR); },
            }
        }

        if done {
            let parsed = string.parse::<u32>();
            if parsed.is_ok() {
                return Some(parsed.unwrap());
            } else {
                change_to_color(minibuffer, ERROR_COLOR);
            }
        }
    }
}

#[allow(unused_variables, unused_assignments)]
fn open_file(win: WINDOW, screen_height: i32, screen_width: i32,
             file_must_exists: bool) -> Option<PathBuf> {
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

        change_to_color(win, NORMAL_COLOR);

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
                        change_to_color(win, ERROR_COLOR);
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
                        change_to_color(win, ERROR_COLOR);
                    }
                } else {
                    change_to_color(win, ERROR_COLOR);
                }
            }
        }

        if done {
            if !do_open_file {
                return None;
            } else {
                if let Ok(path_buf) = handle_file_opening(&string,
                                                          file_must_exists) {
                    return Some(path_buf);
                } else {
                    change_to_color(win, ERROR_COLOR);
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
fn handle_file_opening(string: &String,
                       file_must_exists: bool) -> Result<PathBuf, ()> {
    // TODO(erick): We already had a PathBuf before,
    // we should not have to construct one here.
    let path = Path::new(string.as_str());

    if file_must_exists {
        let meta_data = std::fs::metadata(path);
        if meta_data.is_err() {
            return Err ( () )
        }

        let meta_data = meta_data.unwrap();
        if !meta_data.is_file() {
            return Err( () )
        }
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
    change_to_color(win, NORMAL_COLOR);
    wclear(win);
    wrefresh(win);
}

#[inline]
fn change_to_color(window: WINDOW, color: i16) {
    wbkgd(window, COLOR_PAIR(color));
}

#[inline]
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
                     selected_operation: isize) {
    wmove(window, 0, 0);
    change_to_color(window, NORMAL_COLOR);
    wprintw(window, "Operations:");

    // NOTE(erick): If selected_operation is -1 (meaning no operation is
    // selected) selected_number will be zero an no entry will be highlighted.
    let selected_number = selected_operation + 1;
    let mut operation_number = 1;

    for operation in operations {
        wmove(window, operation_number as i32, 0);

        if operation_number == selected_number {
            change_to_color(window, HIGHLIGHT_COLOR);
            wprintw(window, "> ");
            change_to_color(window, NORMAL_COLOR);
        }

        wprintw(window, format!("{}: ", operation_number).as_str());

        match operation {
            &Operation::Open(file_index) => {
                wprintw(window, format!("Open({})",
                                        file_stem(&opened_files[file_index])).as_str());

            },
            &Operation::Save(op_index, file_index) => {
                wprintw(window, format!("Save({}: {})", op_index,
                                        file_stem(&opened_files[file_index])).as_str());
            },
            _  => {
                wprintw(window, format!("{}", operation).as_str());
            },
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
