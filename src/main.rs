// `agb` を利用するゲームは no_std 、つまり Rust の標準ライブラリを利用できない。
// ゲームボーイアドバンスには OS が搭載されておらず、ほとんどの標準ライブラリは
// 適用できない為である。
//
// 無効にしなければ agb はアロケータを提供するので、
// `core` と `alloc` の両方のビルトインクレートを使うことができる。
#![no_std]
// `agb` は独自の `main` 関数を定義するため、
// ゲームのメイン関数は #[agb::entry] 手続きマクロで宣言する必要がある。
// これに従わない場合はリンクに失敗することになり、
// エラーメッセージからは原因を判断しづらいだろう。
#![no_main]
// これらはテストを書くために必要。
#![cfg_attr(test, feature(custom_test_frameworks))]
#![cfg_attr(test, reexport_test_harness_main = "test_main")]
#![cfg_attr(test, test_runner(agb::test_runner::test_runner))]
#![deny(clippy::all)]

extern crate alloc;

use agb::{
    display::{
        object::{Graphics, Sprite, Tag, TagMap},
        tiled::{RegularBackgroundSize, TileFormat, TiledMap},
        Priority, WIDTH,
    },
    input::Button,
};

agb::include_background_gfx!(tiles,
    "ff00ff", // 透過色p
    bg => "gfx/bg.png");

const GRAPHICS: &Graphics = agb::include_aseprite!("gfx/sprites.aseprite");
const TAG_MAP: &TagMap = GRAPHICS.tags();

const IDLE: &Tag = TAG_MAP.get("Idle");
const WALKING: &Tag = TAG_MAP.get("Walking");
const JUMPING: &Tag = TAG_MAP.get("Jumping");
const APPLE: &Tag = TAG_MAP.get("Apple");
const WINDOW: &Tag = TAG_MAP.get("Window");

fn rgb5(r: u8, g: u8, b: u8) -> u16 {
    let (r, g, b) = (r as u16, g as u16, b as u16);
    (r) | ((g) << 5) | ((b) << 10)
}

// 過去実装で OBJ_CHAR (ATTR2_ID) で表現していた部分の互換処理
fn sprite_for_char(ch: u16) -> &'static Sprite {
    match ch {
        2 => WALKING.sprite(0),
        4 => WALKING.sprite(2),
        6 => JUMPING.sprite(0),
        8 => JUMPING.sprite(1),
        10 => JUMPING.sprite(2),
        _ => IDLE.sprite(0),
    }
}

// メイン関数は1つの引数を取り、値を返さない。
// agb::entry 修飾子によって全てがお膳立てされる。
// `agb` によってスタックとインタラプトハンドラのセットアップが正常に完了した時点で呼ばれる。
// 関数内で利用するための `Gba` 構造体の生成も行われる。
#[agb::entry]
fn main(mut gba: agb::Gba) -> ! {
    let vblank = agb::interrupt::VBlank::get();
    // グラフィックスモード 0
    let (gfx, mut vram) = gba.display.video.tiled0();
    let mut input = agb::input::ButtonController::new();
    // https://www.coranac.com/tonc/text/regbg.htm#ssec-ctrl-bgs
    let mut bg0 = gfx.background(
        Priority::P0,                           // BG0
        RegularBackgroundSize::Background32x32, // BG_REG_32x32
        TileFormat::FourBpp,                    // BG_4BPP 16 色
    );
    vram.set_background_palettes(tiles::PALETTES);
    vram.set_background_palette_colour(
        0, // パレットバンク番号
        0, // パレット内の色番号
        rgb5(15, 15, 31),
    );

    /* ドロイド君 */
    let (mut dx, mut dy) = (120, 120);
    let object = gba.display.object.get_managed();
    let mut droid_object = object.object_sprite(IDLE.sprite(0));
    droid_object.set_position((dx, dy)).set_z(0).show();
    /* りんご */
    let (ax, ay) = (160, 120);
    let ax_range = (ax - 12)..=(ax + 12);
    let a_top_y = ay - 13;
    let mut apple_object = object.object_sprite(APPLE.sprite(0));
    apple_object.set_position((ax, ay)).set_z(1).show();
    /* 窓 */
    let mut window_object = object.object_sprite(WINDOW.sprite(0));
    window_object.set_position((40, 40)).set_z(1).show();

    object.commit();

    /* BG0 をセット */
    let tileset = &tiles::bg.tiles;
    bg0.set_tile(
        &mut vram,
        (0u16, 17u16),
        tileset,
        tiles::bg.tile_settings[5 * 32],
    );
    bg0.set_tile(
        &mut vram,
        (29u16, 17u16),
        tileset,
        tiles::bg.tile_settings[2 + 5 * 32],
    );
    for i in 1..29 {
        bg0.set_tile(
            &mut vram,
            (i, 17u16),
            tileset,
            tiles::bg.tile_settings[1 + 5 * 32],
        );
    }
    for xx in 0u16..30 {
        for yy in 18u16..32 {
            bg0.set_tile(
                &mut vram,
                (xx, yy),
                tileset,
                tiles::bg.tile_settings[3 + 5 * 32],
            );
        }
    }
    bg0.commit(&mut vram);
    bg0.set_visible(true);

    /*
     * ドロイド君の状態。
     * 0 => 待機
     * 1 => ジャンプ準備中
     * 2 => ジャンプ中
     */
    let mut state = 0u8;
    /* ドロイド君の y 方向の速度 */
    let mut vy = 0f32;

    /* フレーム数用の変数 */
    let mut f = 0u16;
    /* 表示するキャラクタ */
    let mut ch = 0u16;
    /* 歩き状態 (0, 1, 2) */
    let mut wstate = 0u8;

    /* メインループ */
    loop {
        /* VBLANK 割り込み待ち */
        vblank.wait_for_vblank();
        /* キー状態取得 */
        input.update();

        match state {
            /* 待機中 */
            0 if input.is_just_pressed(Button::UP) => {
                // ジャンプ開始
                state = 1;
                f = 0;
                ch = 0;
            }
            0 => {
                if input.is_pressed(Button::LEFT) {
                    dx -= 1;
                    droid_object.set_hflip(true);
                }
                if input.is_pressed(Button::RIGHT) {
                    dx += 1;
                    droid_object.set_hflip(false);
                }
                if dx < -16 {
                    dx = WIDTH;
                }
                if input.is_just_pressed(Button::LEFT) || input.is_just_pressed(Button::RIGHT) {
                    wstate = 0;
                    f = 0;
                }
                if input.is_just_released(Button::LEFT) || input.is_just_released(Button::RIGHT) {
                    ch = 0;
                }
                if WIDTH < dx {
                    dx = -16;
                }
                if input.is_pressed(Button::LEFT) || input.is_pressed(Button::RIGHT) {
                    /* 歩きモーション */
                    f += 1;
                    if 5 < f {
                        match wstate {
                            0 => {
                                wstate = 1;
                                ch = 2;
                            }
                            1 => {
                                wstate = 2;
                                ch = 0;
                            }
                            2 => {
                                wstate = 3;
                                ch = 4
                            }
                            _ => {
                                wstate = 0;
                                ch = 0;
                            }
                        }
                        f = 0
                    }
                }
                if dy == a_top_y && !ax_range.contains(&dx) {
                    /* りんごから落ちる */
                    vy = -0.;
                    state = 2;
                    wstate = 0;
                } else {
                    droid_object
                        .set_position((dx, dy))
                        .set_sprite(object.sprite(sprite_for_char(ch)));
                }
            }
            1 | 3 => {
                /* ジャンプ準備 */
                droid_object.set_sprite(object.sprite(sprite_for_char(6)));
                f += 1;
                if 3 < f {
                    vy = 4.;
                    if 1 == state {
                        state = 2
                    } else {
                        state = 4
                    };
                }
            }
            2 if input.is_just_pressed(Button::UP) => {
                /* 二段ジャンプ */
                state = 3;
                f = 0;
            }
            2 | 4 => {
                /* ジャンプ中 */
                if input.is_pressed(Button::LEFT) {
                    dx -= 1;
                    droid_object.set_hflip(true);
                }
                if input.is_pressed(Button::RIGHT) {
                    dx += 1;
                    droid_object.set_hflip(false);
                }
                if dx < -16 {
                    dx = WIDTH;
                }
                if WIDTH < dx {
                    dx = -16;
                }
                if 0.5 < vy && input.is_pressed(Button::UP) {
                    vy += 0.2;
                }
                dy -= vy as i32;
                if vy < 0. {
                    droid_object.set_sprite(object.sprite(sprite_for_char(10)));
                } else {
                    droid_object.set_sprite(object.sprite(sprite_for_char(8)));
                }
                if dy < 0 {
                    dy = 0;
                    vy = -0.;
                }
                if (vy < 0.) && ax_range.contains(&dx) && ( a_top_y < dy ) {
                    /* りんごに乗る */
                    dy = a_top_y;
                    state = 0;
                }
                if 120 < dy {
                    /* 着地 */
                    dy = 120;
                    state = 0;
                }
                droid_object.set_position((dx, dy));
                vy -= 0.3;
            }
            _ => {}
        }
        object.commit();
    }
}
