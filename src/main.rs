extern crate ncurses;
extern crate nix;

use std::fs::ReadDir;
use std::fs::DirEntry;
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

const KEY_Q         : i32 = 'q' as i32;
const KEY_O         : i32 = 'o' as i32;
const KEY_TAB       : i32 = 0x09;
const KEY_ENTER     : i32 = 0x0a;
const KEY_BACKSPACE : i32 = 0x7f;
const KEY_ESC       : i32 = 0x1b;

extern fn stop_program(_: i32) {
    endwin();
    std::process::exit(0);
}

const ERROR_COLOR : i16 = 1;

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

    let screen_width  = getmaxx(stdscr());
    let mut screen_height = getmaxy(stdscr());

    // NOTE(erick): Creating space for the minibuffer.
    wresize(stdscr(), screen_height - 1, screen_width);
    screen_height -= 1;

    let minibuffer_window = newwin(1, screen_width, screen_height, 0);

    let mut opened_files = Vec::new();
    loop {
        wprint_strings(stdscr(), &opened_files);
        clear_window(minibuffer_window);
        refresh();

        let ch = getch();
        match ch {
            KEY_Q => { break; },
            KEY_O => {
                let new_file = open_file(minibuffer_window);
                if new_file.is_some() {
                    opened_files.push(new_file.unwrap());
                }
            },

            _     => { print_i32_char(ch); },
        };
    }

    endwin();
}

fn wprint_strings(win: WINDOW, strings: &Vec<String>) {
    let mut line_numer = 0;
    for string in strings {
        wmove(win, line_numer, 0);
        wprintw(win, string.as_ref());

        line_numer += 1;
    }
}

fn clear_window(win: WINDOW) {
    wbkgd(win, COLOR_PAIR(0));
    wclear(win);
    wrefresh(win);
}

#[allow(unused_variables, unused_assignments)]
fn open_file(win: WINDOW) -> Option<String> {
    let mut string = get_current_path();

    let mut done = false;
    let mut do_open_file = false;
    let mut is_autocompleting = false;
    let mut completion = None;
    let mut completion_index = 0;

    while !done {
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
                completion = get_maximum_path_matching(string.as_ref());
            }

            // TODO(erick): Else we could paint the buffer red
            if completion.is_some() {
                let options = completion.as_ref().unwrap();
                if options.len() > 0 {
                    string = options[completion_index].clone();
                    completion_index += 1;
                    if completion_index >= options.len() {
                        completion_index = 0;
                    }
                }
            } else {
                wbkgd(win, COLOR_PAIR(ERROR_COLOR));
            }

            is_autocompleting = true;
        }

        if restart_state {
            is_autocompleting = false;
            completion_index = 0;
        }
    }

    Some(string)
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

fn get_char(ch: i32) -> char {
    char::from_u32(ch as u32).expect("Invalid char")
}

fn print_i32_char(ch: i32) {
    wprint_i32_char(stdscr(), ch);
}

fn wprint_i32_char(win: WINDOW, ch: i32) {
    wprintw(win, format!("{}", get_char(ch)).as_ref());
}

#[allow(dead_code)]
fn wprint_i32_char_debug(win: WINDOW, ch: i32) {
    wprintw(win, format!("{}: {:08x}", get_char(ch), ch).as_ref());
}
