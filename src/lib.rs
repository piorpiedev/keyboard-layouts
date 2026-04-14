#![no_std]
extern crate log;

use core::fmt;

use gen_layouts_sys::*;
use heapless::Vec;

pub use gen_layouts_sys::Layout;

const UNICODE_ENTER: u16 = 10; // \n
const UNICODE_TAB: u16 = 9; // \t
// https://stackoverflow.com/questions/23320417/what-is-this-character-separator
const CONTROL_CHARACTER_OFFSET: u16 = 0x40;
const UNICODE_FIRST_ASCII: u16 = 0x20; // SPACE
const UNICODE_LAST_ASCII: u16 = 0x7F; // BACKSPACE
const KEY_MASK: u16 = 0x3F; // Remove SHIFT/ALT/CTRL from keycode
/// The number of bytes in a keyboard HID packet
pub const HID_PACKET_LEN: usize = 8;
const RELEASE_KEYS_HID_PACKET: [u8; 8] = [0; 8];

const MAX_MODIFIER_KEYS: usize = 8;

#[derive(Debug)]
pub enum Error {
    InvalidLayoutKey,
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum Release {
    All = 0,
    Keys = 1,
    None = 2,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyMod {
    pub key: u8,
    pub modifier: u8,
    pub release: Release,
}

enum Keycode {
    ModifierKeySequence(u16, Vec<u16, MAX_MODIFIER_KEYS>),
    RegularKey(u16),
    InvalidCharacter,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::InvalidLayoutKey => write!(f, "Invalid keyboard layout"),
        }
    }
}

/// Get a list of the supported keyboard layouts
pub fn available_layouts() -> Vec<&'static str, LAYOUTS_NUM> {
    LAYOUT_MAP.iter().map(|(n, _)| n).cloned().collect()
}

/// Get keyboard layout reference
pub fn get_layout(layout_key: &str) -> Result<&Layout, Error> {
    let (_, layout) = LAYOUT_MAP
        .iter()
        .find(|(n, _)| *n == layout_key)
        .ok_or(Error::InvalidLayoutKey)?;

    Ok(layout)
}

/// Get a list of the key and modifier pairs required to type the given string on a keyboard with
/// the specified layout.
pub fn string_to_keys_and_modifiers<'a>(
    layout: &'a Layout,
    string: &'a str,
) -> impl Iterator<Item = KeyMod> + 'a {
    string.chars().flat_map(move |c| {
        let mut char_reports: Vec<KeyMod, { MAX_MODIFIER_KEYS + 1 }> = Vec::new(); // Adding one for the release packet

        match keycode_for_unicode(layout, c as u16) {
            Keycode::ModifierKeySequence(modifier, sequence) => {
                for keycode in sequence {
                    char_reports
                        .push(KeyMod {
                            key: keycode as u8,
                            modifier: modifier as u8,
                            release: Release::Keys,
                        })
                        .expect("sequence is intinialized with capacity MAX_MODIFIER_KEYS +1");
                }
                // Manually add release after sequence is finished
                char_reports
                    .push(KeyMod {
                        key: 0,
                        modifier: 0,
                        release: Release::None,
                    })
                    .expect("we left +1 in capacity so we must be able to add an extra report");
            }
            Keycode::RegularKey(keycode) => {
                if let Some(dead_keycode) = deadkey_for_keycode(layout, keycode) {
                    let key = key_for_keycode(layout, dead_keycode);
                    let modifier = modifier_for_keycode(layout, dead_keycode);
                    char_reports
                        .push(KeyMod {
                            key,
                            modifier,
                            release: Release::All,
                        })
                        .expect("MAX_MODIFIER_KEYS must be a positive number, greater than 1");
                }
                let key = key_for_keycode(layout, keycode);
                let modifier = modifier_for_keycode(layout, keycode);
                char_reports
                    .push(KeyMod {
                        key,
                        modifier,
                        release: Release::All,
                    })
                    .expect("MAX_MODIFIER_KEYS must be a positive number, greater than 1");
            }
            _ => {}
        }

        char_reports
    })
}

/// Create the sequence of HID packets required to type the given string. Impersonating a keyboard
/// with the specified layout. These packets can be written directly to a HID device file.
pub fn string_to_hid_packets<'a>(
    layout: &'a Layout,
    string: &'a str,
) -> impl Iterator<Item = [u8; 8]> {
    string_to_keys_and_modifiers(layout, string).flat_map(move |m| {
        let mut packets: Vec<[u8; 8], 2> = Vec::new();
        packets
            .push([m.modifier, 0, m.key, 0, 0, 0, 0, 0])
            .expect("1 < 2, so there is space");
        match m.release {
            Release::All => {
                packets
                    .push(RELEASE_KEYS_HID_PACKET)
                    .expect("2 <= 2, so there is space");
            }
            Release::Keys => {
                packets
                    .push([m.modifier, 0, 0, 0, 0, 0, 0, 0])
                    .expect("2 <= 2, so there is space");
            }
            Release::None => {}
        }
        packets
    })
}

fn keycode_for_unicode(layout: &Layout, unicode: u16) -> Keycode {
    match unicode {
        u if u == UNICODE_ENTER => Keycode::RegularKey(ENTER_KEYCODE & layout.keycode_mask),
        u if u == UNICODE_TAB => Keycode::RegularKey(TAB_KEYCODE & layout.keycode_mask),
        u if u < UNICODE_FIRST_ASCII => {
            let idx = ((u + CONTROL_CHARACTER_OFFSET) - UNICODE_FIRST_ASCII) as usize;
            let mut keycodes = Vec::new();
            keycodes.push(layout.keycodes[idx]).expect("one is less than 8, so we must always be able to insert into the vec, unless capacity (MAX_MODIFIER_KEYS) is set to 0");
            Keycode::ModifierKeySequence(RIGHT_CTRL_MODIFIER, keycodes)
        }
        u if (UNICODE_FIRST_ASCII..=UNICODE_LAST_ASCII).contains(&u) => {
            let idx = (u - UNICODE_FIRST_ASCII) as usize;
            Keycode::RegularKey(layout.keycodes[idx])
        }
        _ => Keycode::InvalidCharacter,
    }
}

// https://github.com/PaulStoffregen/cores/blob/master/teensy3/usb_keyboard.c
fn deadkey_for_keycode(layout: &Layout, keycode: u16) -> Option<u16> {
    layout.dead_keys_mask.and_then(|dkm| {
        let keycode = keycode & dkm;
        if let Some(acute_accent_bits) = layout.deadkeys.acute_accent_bits
            && keycode == acute_accent_bits
        {
            return layout.deadkeys.deadkey_accute_accent;
        }
        if let Some(cedilla_bits) = layout.deadkeys.cedilla_bits
            && keycode == cedilla_bits
        {
            return layout.deadkeys.deadkey_cedilla;
        }
        if let Some(diaeresis_bits) = layout.deadkeys.diaeresis_bits
            && keycode == diaeresis_bits
        {
            return layout.deadkeys.deadkey_diaeresis;
        }
        if let Some(grave_accent_bits) = layout.deadkeys.grave_accent_bits
            && keycode == grave_accent_bits
        {
            return layout.deadkeys.deadkey_grave_accent;
        }
        if let Some(circumflex_bits) = layout.deadkeys.circumflex_bits
            && keycode == circumflex_bits
        {
            return layout.deadkeys.deadkey_circumflex;
        }
        if let Some(tilde_bits) = layout.deadkeys.tilde_bits
            && keycode == tilde_bits
        {
            return layout.deadkeys.deadkey_tilde;
        }
        None
    })
}

// https://github.com/PaulStoffregen/cores/blob/master/usb_hid/usb_api.cpp#L196
fn modifier_for_keycode(layout: &Layout, keycode: u16) -> u8 {
    let mut modifier = 0;

    if keycode & layout.shift_mask > 0 {
        modifier |= SHIFT_MODIFIER;
    }

    if let Some(alt_mask) = layout.alt_mask
        && keycode & alt_mask > 0
    {
        modifier |= RIGHT_ALT_MODIFIER;
    }

    if let Some(ctrl_mask) = layout.ctrl_mask
        && keycode & ctrl_mask > 0
    {
        modifier |= RIGHT_CTRL_MODIFIER;
    }

    modifier as u8
}

// https://github.com/PaulStoffregen/cores/blob/master/usb_hid/usb_api.cpp#L212
fn key_for_keycode(layout: &Layout, keycode: u16) -> u8 {
    let key = keycode & KEY_MASK;
    match layout.non_us {
        Some(non_us) => {
            if key == non_us {
                100
            } else {
                key as u8
            }
        }
        None => key as u8,
    }
}
