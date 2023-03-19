use crate::m3u::{PlaylistGroup, PlaylistItem, PlaylistItemHeader};

fn token_value(it: &mut std::str::Chars) -> String {
    if let Some(oc) = it.next() {
        if oc == '"' {
            return get_value(it);
        }
    }
    String::from("")
}

fn get_value(it: &mut std::str::Chars) -> String {
    let mut result: Vec<char> = vec![];
    while let Some(oc) = it.next() {
        if oc == '"' {
            break;
        }
        result.push(oc);
    }
    String::from(result.iter().collect::<String>())
}

fn token_till(it: &mut std::str::Chars, stop_char: char) -> Option<String> {
    let mut result: Vec<char> = vec![];
    loop {
        match it.next() {
            Some(ch) => {
                if ch == stop_char {
                    break;
                } else if ch.is_whitespace() && result.is_empty() {
                    continue;
                } else {
                    result.push(ch);
                }
            }
            None => break,
        }
    }
    if !result.is_empty() { Some(String::from(result.iter().collect::<String>())) } else { None }
}

fn skip_digit(it: &mut std::str::Chars) -> Option<char> {
    loop {
        match it.next() {
            Some(c) => {
                if !(c == '-' || c == '+' || c.is_digit(10)) {
                    return Some(c);
                }
            }
            None => return None,
        }
    }
}

fn decode_header(content: &String) -> PlaylistItemHeader {
    let mut plih = PlaylistItemHeader {
        id: String::from(""),
        name: String::from(""),
        logo: String::from(""),
        logo_small: String::from(""),
        group: String::from("Unknown"),
        title: String::from(""),
        parent_code: String::from(""),
        audio_track: String::from(""),
        time_shift: String::from(""),
        rec: String::from(""),
        source: String::from(content),
        chno: String::from(""),
    };

    let mut it = content.chars();
    let line_token = token_till(&mut it, ':');
    if line_token == Some(String::from("#EXTINF")) {
        let mut c = skip_digit(&mut it);
        loop {
            if !c.is_some() {
                break;
            }
            match c.unwrap() {
                ',' => plih.title = get_value(&mut it),
                _ => {
                    let token = token_till(&mut it, '=');
                    if token != None {
                        let value = token_value(&mut it);
                        match token.unwrap().as_str() {
                            "tvg-id" => plih.id = value,
                            "tvg-name" => plih.name = value,
                            "group-title" => if !value.is_empty() { plih.group = value },
                            "parent-code" => plih.parent_code = value,
                            "audio-track" => plih.audio_track = value,
                            "tvg-logo" => plih.logo = value,
                            "tvg-logo-small" => plih.logo_small = value,
                            "timeshift" => plih.time_shift = value,
                            "tvg-rec" => plih.rec = value,
                            _ => {}
                        }
                    }
                }
            }
            c = it.next();
        }
    }
    plih
}

pub(crate) fn decode(lines: &Vec<String>) -> Vec<PlaylistGroup> {
    let mut groups: std::collections::HashMap<String, Vec<PlaylistItem>> = std::collections::HashMap::new();
    let mut sort_order: Vec<String> = vec![];
    let mut header: Option<String> = None;
    let mut group: Option<String> = None;

    for line in lines {
        if line.starts_with("#EXTINF") {
            header = Some(String::from(line));
            continue;
        }
        if line.starts_with("#EXTGRP") {
            group = Some(String::from(&line[8..]));
            continue;
        }
        if line.starts_with("#") {
            continue;
        }
        if header.is_some() {
            let mut item = PlaylistItem { header: decode_header(&header.unwrap()), url: String::from(line) };
            if group.is_some() && item.header.group.is_empty() {
                item.header.group = String::from(group.unwrap());
            }
            let key = String::from(&item.header.group);
            // let key2 = String::from(&item.header.group);
            match groups.entry(key.clone()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(vec![item]);
                    sort_order.push(key);
                }
                std::collections::hash_map::Entry::Occupied(mut e) => { e.get_mut().push(item); }
            }
        }
        header = None;
        group = None;
    }

    let mut result: Vec<PlaylistGroup> = vec![];
    for (key, value) in groups {
        result.push(PlaylistGroup { title: key, channels: value });
    }
    result.sort_by(|f, s| {
        let i1 = sort_order.iter().position(|r| r == &f.title).unwrap();
        let i2 = sort_order.iter().position(|r| r == &s.title).unwrap();
        return i1.cmp(&i2);
    });
    result
}
