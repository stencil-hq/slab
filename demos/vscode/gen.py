#!/usr/bin/env python3
"""Generates vscode.slab — the VS Code demo document (chrome, defs, params).
Editors, file tree, and syntax highlighting are host-driven at runtime
(web/main.js, host.ts, crates/slab-native/src/vscode.rs). Source of truth for
the demo; edit here, then:
    python3 demos/vscode/gen.py && cargo run -q -p slab-cli -- render demos/vscode/vscode.slab -o /tmp/vscode.png --width 1568 --height 844
"""

# ---------------------------------------------------------------- palette ----
C = dict(
    bg        = '#0C0B0E',   # chrome / editor background
    bg2       = '#0D0C0F',   # sidebar / activity / chat
    bgDark    = '#09090B',   # focused editor bg
    crumbBg   = '#0A0A0C',
    border    = '#1E1E22',
    sep       = '#161519',
    txt       = '#B6B7BE',   # default code text
    ui        = '#909094',   # bright ui text
    uiDim     = '#707074',   # tree files, statusbar
    uiFaint   = '#55555A',
    kw        = '#9D74BE',   # keywords / preproc
    fn        = '#5E9AA0',   # function calls
    typ       = '#DCB99E',   # types / numbers
    str_      = '#DCA9A8',   # strings
    com       = '#46454A',   # comments
    red       = '#A63D5B',   # error filenames / badges
    redDim    = '#7A3247',
    rose      = '#6A3E4D',   # active-tab underline
    blue      = '#3C7BAC',   # h++ file icon
    sel       = '#242431',   # find widget / selection block
    findBg    = '#252531',
    inputBg   = '#131217',
    cardBg    = '#141318',
    yellow    = '#D9A33C',
    tlRed     = '#FF5F57', tlYel = '#FEBC2E', tlGrn = '#28C840',
)


def esc(s: str) -> str:
    return s.replace('\\', '\\\\').replace('"', '\\"')


# Editor metrics: 12px JetBrains Mono on an 18px line grid (VS Code defaults).
CODE_SIZE = 12
CODE_LINE_H = 18

def find_widget(status, close_extra='', field_extra=''):
    """The floating editor find bar in the EGroup editor stack."""
    return f"""row h=28 radius=4 bg={C['findBg']} self=top-end offset=-60,2 align=center pad=0,6 gap=6 shadow=0,2,8,#00000066 {{
            icon chev-r size=8 color=color.uiDim
            row w=140 h=20 radius=3 bg=#1B1B24 align=center pad=0,6 {{
              text "" field=find_change{field_extra} w=fill size=12 color=#D5D5DA nowrap label="Find"
            }}
            text "Aa" size=11 color=color.uiDim nowrap
            text "ab" size=11 color=color.uiDim underline nowrap
            text ".*" size=11 color=color.uiDim nowrap
            text {status} size=11 color={C['red']} nowrap
            text "↑" size=11 color=color.uiFaint nowrap
            text "↓" size=11 color=color.uiFaint nowrap
            icon lines size=11 color=color.uiDim
            icon close size=10 color=color.ui{close_extra}
          }}"""


# ------------------------------------------------------------ file tree -----
# (name, depth, kind, tint, badge)  kind: dir-open, dir-closed, h, c, misc
TREE = [
 ('build',        0, 'dc', None, None),
 ('include',      0, 'do', None, None),
 ('agentfs',      1, 'do', 'red', None),
 ('cid.hpp',      2, 'h',  None, None),
 ('launch.hpp',   2, 'h',  None, None),
 ('mdb.hpp',      2, 'h',  'red', '2'),
 ('node.hpp',     2, 'h',  None, None),
 ('overlay.hpp',  2, 'h',  None, None),
 ('path.hpp',     2, 'h',  None, None),
 ('store.hpp',    2, 'h',  'red', '2'),
 ('scripts',      0, 'dc', None, None),
 ('src',          0, 'do', None, None),
 ('cli',          1, 'do', None, None),
 ('main.cpp',     2, 'c',  None, None),
 ('core',         1, 'do', None, None),
 ('launch.cpp',   2, 'c',  None, None),
 ('mdb.cpp',      2, 'c',  None, None),
 ('overlay.cpp',  2, 'c',  None, None),
 ('path.cpp',     2, 'c',  None, None),
 ('store.cpp',    2, 'c',  None, None),
 ('hook',         1, 'do', None, None),
 ('context.cpp',  2, 'c',  None, None),
 ('hook.hpp',     2, 'h',  None, None),
 ('hooks_path.cpp',2,'c',  None, None),
 ('hooks_proc.cpp',2,'c',  None, None),
 ('interpose.hpp',2, 'h',  None, None),
 ('spawn.cpp',    2, 'c',  None, None),
 ('spawn.hpp',    2, 'h',  None, None),
 ('interpose',    1, 'dc', None, None),
 ('tests',        0, 'dc', None, None),
 ('third_party',  0, 'dc', None, None),
 ('.clang-format',0, 'm',  None, None),
 ('.gitignore',   0, 'm',  None, None),
 ('CMakeLists.txt',0,'m',  None, None),
 ('demo.sh',      0, 'm',  None, None),
 ('README.md',    0, 'm',  None, None),
]

def tree_defaults():
    icons = {
        '.clang-format': 'gearfile',
        '.gitignore': 'branch',
        'CMakeLists.txt': 'cmakefile',
        'demo.sh': 'shellfile',
        'README.md': 'markdown',
    }
    out = []
    for name, depth, kind, tint, badge in TREE:
        color = C['red'] if tint == 'red' else (C['ui'] if kind.startswith('d') else C['uiDim'])
        letter = 'h' if kind == 'h' else ('C' if kind == 'c' else '')
        icon = icons.get(name, 'blank')
        if kind == 'do':
            icon = 'folder-open'
        elif kind == 'dc':
            icon = 'folder'
        out.append(
            f'    TreeRow(name="{esc(name)}", letter="{letter}", icon="{icon}", '
            f'tint={color}, badge="{badge or ""}", indent={14 + depth * 10}, '
            f'dir={"true" if kind.startswith("d") else "false"}, '
            f'open={"true" if kind == "do" else "false"})'
        )
    return ',\n'.join(out)


TREE_ROW_DEF = f'''icon blank viewbox=24 {{ path "M0 0" }}
def TreeRow(name="", letter="", icon="", tint=#707074, badge="", indent=14, dir=false, open=false) export {{
  row h=22 gap=4 align=center pad-l=indent act=tree_pick label="tree row" {{
    when hover {{ bg=#FFFFFF08 }}
    when selected {{ bg=#1D1C22 }}
    stack w=10 h=10 {{
      icon chev-d size=10 color=#00000000 {{ when open {{ color=#55555A }} }}
      icon chev-r size=10 color=#00000000 {{ when dir {{ color=#55555A }} when open {{ color=#00000000 }} }}
    }}
    text letter size=11 weight=700 color=#3C7BAC nowrap family="JetBrains Mono"
    icon icon size=16 color=#55555A
    text name size=13 color=tint nowrap
    spacer
    text badge size=11 color=#A63D5B nowrap
    rect w=10 h=1 bg=none
  }}
}}

params {{
  tree list(TreeRow) = [
{tree_defaults()}
  ]
}}'''

# ---------------------------------------------------------------- tabs ------
# EditorTab is the interactive VS Code tab template shared by both groups.
# Signals (host implements VS Code semantics over SDP):
#   tab_press      switch on primary mousedown (skip when hit_key is the close icon)
#   tab_up         middle-button close (meta.button == 1)
#   tab_close      close via the X icon
#   tab_dbl        double-click a tab (pin) / strip_new on empty strip space
#   tab_drag/_move/_end   drag lifecycle; drag-ghost paints the drag image
#   tab_drop       drop on a tab; host applies the VS Code midpoint rule
#   strip_drop     drop on empty strip space appends to that group's end
# Host-driven node states: insert-before / insert-after on #indl/#indr rects,
# hot (underline) and active come from list fields.
# ---- editor grid template pieces -------------------------------------------


CRUMB_DEF = f'''def Crumb(seg="", letter="", last=false) export {{
  row gap=5 align=center {{
    text letter size=11 weight=700 color=#3C7BAC nowrap family="JetBrains Mono"
    text seg size=12 color=#707074 nowrap {{
      when last {{ color=#909094 }}
    }}
    icon chev-r size=8 color=#55555A {{
      when last {{ color=#00000000 }}
    }}
  }}
}}'''

# Recursive editor-group grid: one def models both branch nodes (axis via
# `horizontal`, panes in `children`) and leaf groups (tab strip + breadcrumb +
# editor). The kernel `splits` container owns pane ratios and sashes; the host
# applies VS Code grid.ts semantics (insert sibling on same orientation, wrap
# on different, Sizing.Split halves the reference pane).
EGROUP_DEF = f'''def EGroup(leaf=false, horizontal=false, show_mdb=false, show_store=false, show_edit=false, show_find=false, find_status="No results", curline=0, curline_on=false, gutter="", crumbs=list(Crumb), tabs=list(EditorTab), children=list(EGroup)) export {{
  stack w=fill h=fill {{
    col #panes w=fill h=fill gap=0 splits split-w=4 split-fg=#3C7BACAA resize=split_resize label="Editor groups" {{
      when horizontal {{ axis=row }}
      when leaf {{ max-w=0 max-h=0 }}
      each children #kids
    }}
    col #chrome w=fill h=fill gap=0 {{
      when !leaf {{ max-w=0 max-h=0 }}
      when leaf {{
      row h=30 bg=color.bg gap=0 {{
        row #strip w=fill h=fill scroll scrollbar=auto scrollbar-w=3 scrollbar-fg=#42424566 drop=strip_drop dblclick=strip_dbl label="Tab strip" {{
          each tabs #tabs
        }}
        icon expand size=11 color=color.uiDim self=center
        rect w=10 h=1 bg=none
        icon square2 size=11 color=color.uiDim self=center
        rect w=10 h=1 bg=none
        text "···" size=12 color=color.uiDim nowrap self=center
        rect w=10 h=1 bg=none
      }}
      row #crumb h=22 bg=color.crumbBg align=center pad=0,12 gap=5 {{
        each crumbs #crumbseach
      }}
      stack #ed w=fill h=fill bg=color.bgDark clip drop=editor_drop label="Editor" {{
        when show_mdb {{ bg=color.bg }}
        stack #content w=fill h=fill {{
          when show_edit {{
            col #edscroll w=fill h=fill scroll=both scrollbar=auto scrollbar-w=6 scrollbar-fg=#42424599 scrollbar-bg=#00000000 clip {{
              stack w=hug {{
                col w=fill h=hug pad-t=curline inert {{
                  rect #curline w=fill h={CODE_LINE_H} bg=#00000000 inert {{
                    when curline_on {{ bg=#FFFFFF06 }}
                  }}
                }}
                row w=hug gap=12 pad=8,0,8,14 {{
                  text gutter w=34 align-text=end size={CODE_SIZE} family="JetBrains Mono" leading=1.5 color=#444348
                  text "" field=code_change multiline tab-size=3 nowrap w=hug size={CODE_SIZE} family="JetBrains Mono" leading=1.5 color=#B6B7BE escape-blur label="Editor"
                }}
              }}
            }}
          }}
        }}
        when show_find {{
          {find_widget('find_status', close_extra=' act=find_close label="Close find"', field_extra=' submit=find_submit escape-blur')}
        }}
        rect #zone w=fill h=fill self=top-start bg=none inert transition=70,ease-out {{
          when zone-merge {{ bg=#24243166 }}
          when zone-left {{ bg=#24243166 w=50% }}
          when zone-right {{ bg=#24243166 w=50% self=top-end }}
          when zone-up {{ bg=#24243166 h=50% }}
          when zone-down {{ bg=#24243166 h=50% self=bottom-start }}
        }}
      }}
      }}
    }}
  }}
}}'''


MENU_ROW_DEF = '''def MenuRow(label="", enabled=true) export {
  row h=26 pad=0,10 align=center act=menu_pick label="menu item" {
    when hover { bg=#2D6CB3 }
    text label size=13 color=#C9C9CC nowrap {
      when !enabled { color=#55555A }
    }
  }
}

params menu {
  open bool = false
  anchor text = ""
  items list(MenuRow) = []
}'''


QO_ROW_DEF = '''def QoRow(name="", dir="", letter="", selected=false) export {
  row h=24 pad=0,8 gap=6 align=center act=qo_pick label="file" {
    when selected { bg=#2D6CB3 }
    when hover { bg=#FFFFFF0A }
    text letter size=10 weight=700 color=#4FC3F7 w=14
    text name size=13 color=#E8E8EA nowrap
    text dir size=12 color=#707074 nowrap ellipsis
  }
}

params qo {
  open bool = false
  sel num = 0
  rows list(QoRow) = []
}'''


EDITOR_TAB_DEF = f'''def EditorTab(name="", note="", tint={C['txt']}, badge="", active=false, hot=false, preview=false, dirty=false) export {{
  col w=hug h=30 {{
    stack w=hug h=28 {{
      row #body h=28 gap=5 align=center pad=0,10 \\
          press=tab_press pointer-up=tab_up dblclick=tab_dbl drop=tab_drop context=tab_menu \\
          drag=tab_drag drag-ghost drag-update=tab_move drag-end=tab_end {{
        when hover {{ bg=#FFFFFF06 }}
        text "h" size=11 weight=700 color={C['blue']} nowrap family="JetBrains Mono"
        text name size=13 color=tint nowrap {{ when preview {{ italic=true }} }}
        text note size=11 color={C['uiFaint']} nowrap
        text badge size=11 color={C['red']} nowrap
        text "●" size=10 color=#00000000 nowrap {{ when dirty {{ color=#C9C9CC }} }}
        when active {{ icon close size=10 color={C['ui']} act=tab_close label="Close tab" }}
      }}
      rect #indl w=2 h=28 self=top-start bg=none inert {{ when insert-before {{ bg=#C9C9CC }} }}
      rect #indr w=2 h=28 self=top-end bg=none inert {{ when insert-after {{ bg=#C9C9CC }} }}
    }}
    rect h=2 bg=none {{ when hot {{ bg={C['rose']} }} }}
  }}
}}

{CRUMB_DEF}

{EGROUP_DEF}

params {{
  root list(EGroup) = [
    EGroup(horizontal=true, children=[
      EGroup(leaf=true, show_mdb=true, tabs=[
        EditorTab(name="mdb.hpp", note="agentfs", tint={C['red']}, badge="2", active=true)
      ]),
      EGroup(leaf=true, show_store=true, tabs=[
        EditorTab(name="overlay.hpp", note="agentfs", tint={C['ui']}),
        EditorTab(name="store.hpp", note="agentfs", tint={C['red']}, badge="2", active=true, hot=true)
      ])
    ])
  ]
}}


params nav {{
  canback bool = false
  canfwd bool = false
}}

params status {{
  lang text = "C++"
  caret text = "Ln 1, Col 1"
  errs text = "6"
  warns text = "2"
}}

params panel {{
  problems bool = false
  output bool = true
  debugc bool = false
  terminal bool = false
  ports bool = false
  termlog text = "can@mac slab-lang %"
}}
'''


# ------------------------------------------------------------- document -----
doc = []
A = doc.append

A(f'''// Generated by demos/vscode/gen.py — do not hand edit.
tokens {{
  color {{
    bg {C['bg']}; bg2 {C['bg2']}; bgDark {C['bgDark']}; crumbBg {C['crumbBg']}
    border {C['border']}; sep {C['sep']}
    txt {C['txt']}; ui {C['ui']}; uiDim {C['uiDim']}; uiFaint {C['uiFaint']}
    kw {C['kw']}; fn {C['fn']}; typ {C['typ']}; str {C['str_']}; com {C['com']}
    red {C['red']}; rose {C['rose']}; blue {C['blue']}; sel {C['sel']}
  }}
}}

// Stroked line icons on a 24-unit box. The kernel scales stroke width with
// the icon (side/viewbox), so each stroke-w is sized for the icon's render
// size: stroke-w ~= 1.5px * 24 / size keeps ~1.5 device px at 1x.
icon chev-r viewbox=24 {{ path "M8 4 L17 12 L8 20" bg=none stroke=current stroke-w=4 }}
icon chev-d viewbox=24 {{ path "M4 8 L12 17 L20 8" bg=none stroke=current stroke-w=3.8 }}
icon close viewbox=24 {{ path "M5 5 L19 19 M19 5 L5 19" bg=none stroke=current stroke-w=3.6 }}
// Activity-bar glyphs below are the official filled Codicon outlines from
// microsoft/vscode-codicons, CC BY 4.0 — see LICENSE-codicons-CC-BY-4.0.txt
// beside gen.py. Their built-in outline weight is ~1 unit on a 16 grid /
// 1.5 on 24, i.e. ~1.4px at the 22px render size.
icon search viewbox=16 {{ path "M10.0195 10.7266C9.06578 11.5217 7.83875 12 6.5 12C3.46243 12 1 9.53757 1 6.5C1 3.46243 3.46243 1 6.5 1C9.53757 1 12 3.46243 12 6.5C12 7.83875 11.5217 9.06578 10.7266 10.0195L13.8535 13.1464C14.0488 13.3417 14.0488 13.6583 13.8535 13.8536C13.6583 14.0488 13.3417 14.0488 13.1464 13.8536L10.0195 10.7266ZM11 6.5C11 4.01472 8.98528 2 6.5 2C4.01472 2 2 4.01472 2 6.5C2 8.98528 4.01472 11 6.5 11C8.98528 11 11 8.98528 11 6.5Z" bg=current }}
icon search-sm viewbox=24 {{ path "M10 3 A7 7 0 1 0 10 17 A7 7 0 1 0 10 3 M15.5 15.5 L21 21" bg=none stroke=current stroke-w=3.2 }}
icon branch viewbox=24 {{ path "M7 6 A2.4 2.4 0 1 0 7 10.8 A2.4 2.4 0 1 0 7 6 M7 10.8 L7 16 M7 16 A2.4 2.4 0 1 0 7 20.8 A2.4 2.4 0 1 0 7 16 M17 6 A2.4 2.4 0 1 0 17 10.8 A2.4 2.4 0 1 0 17 6 M17 10.8 C17 14 7 12 7 16" bg=none stroke=current stroke-w=2.7 }}
icon files viewbox=24 {{ path "M7.5 22.5H17.595C17.07 23.4 16.11 24 15 24H7.5C4.185 24 1.5 21.315 1.5 18V6C1.5 4.89 2.1 3.93 3 3.405V18C3 20.475 5.025 22.5 7.5 22.5ZM21 8.121V18C21 19.6545 19.6545 21 18 21H7.5C5.8455 21 4.5 19.6545 4.5 18V3C4.5 1.3455 5.8455 0 7.5 0H12.879C13.4715 0 14.0505 0.24 14.4705 0.6585L20.3415 6.5295C20.766 6.954 21 7.5195 21 8.121ZM13.5 6.75C13.5 7.164 13.8375 7.5 14.25 7.5H19.1895L13.5 1.8105V6.75ZM19.5 18V9H14.25C13.0095 9 12 7.9905 12 6.75V1.5H7.5C6.672 1.5 6 2.1735 6 3V18C6 18.8265 6.672 19.5 7.5 19.5H18C18.828 19.5 19.5 18.8265 19.5 18Z" bg=current }}
icon play viewbox=24 {{ path "M8 5 L19 12 L8 19 Z" bg=none stroke=current stroke-w=2.8 }}
icon warn viewbox=24 {{ path "M12 4 L22 20 L2 20 Z M12 10 L12 15 M12 17.5 L12 18.5" bg=none stroke=current stroke-w=2.4 }}
icon errc viewbox=24 {{ path "M12 3 A9 9 0 1 0 12 21 A9 9 0 1 0 12 3 M6 6 L18 18" bg=none stroke=current stroke-w=2.4 }}
icon gear viewbox=24 {{ path "M12 9C10.3425 9 9.00002 10.3425 9.00002 12C9.00002 13.6575 10.3425 15 12 15C13.6575 15 15 13.6575 15 12C15 10.3425 13.6575 9 12 9ZM12 13.5C11.172 13.5 10.5 12.828 10.5 12C10.5 11.172 11.172 10.5 12 10.5C12.828 10.5 13.5 11.172 13.5 12C13.5 12.828 12.828 13.5 12 13.5ZM21.8475 14.5725L19.9185 12.942C19.8675 12.8985 19.8195 12.8505 19.776 12.7995C19.332 12.279 19.3965 11.5005 19.9185 11.058L21.8475 9.4275C22.0395 9.2655 22.113 9.0045 22.0365 8.766C21.579 7.3545 20.823 6.06 19.8285 4.962C19.7085 4.83 19.5405 4.758 19.368 4.758C19.2975 4.758 19.227 4.77 19.1595 4.794L16.779 5.6415C16.716 5.664 16.65 5.682 16.584 5.694C16.509 5.7075 16.434 5.715 16.3605 5.715C15.7725 5.715 15.2505 5.298 15.141 4.701L14.6865 2.223C14.6415 1.977 14.451 1.782 14.205 1.7295C13.485 1.5765 12.7485 1.5 12.0015 1.5C11.2545 1.5 10.5165 1.578 9.79652 1.7295C9.55052 1.782 9.36002 1.977 9.31502 2.223L8.86202 4.701C8.85002 4.767 8.83202 4.8315 8.80952 4.8945C8.62802 5.4 8.15102 5.715 7.64102 5.715C7.50302 5.715 7.36202 5.691 7.22402 5.643L4.84352 4.7955C4.77602 4.7715 4.70402 4.7595 4.63502 4.7595C4.46252 4.7595 4.29452 4.8315 4.17452 4.9635C3.17852 6.0615 2.42402 7.356 1.96502 8.7675C1.88702 9.006 1.96202 9.267 2.15402 9.429L4.08302 11.0595C4.13402 11.103 4.18202 11.151 4.22552 11.202C4.66952 11.7225 4.60502 12.501 4.08302 12.9435L2.15402 14.574C1.96202 14.736 1.88852 14.997 1.96502 15.2355C2.42252 16.647 3.17852 17.9415 4.17452 19.0395C4.29452 19.1715 4.46252 19.2435 4.63502 19.2435C4.70552 19.2435 4.77602 19.2315 4.84352 19.2075L7.22402 18.36C7.28702 18.3375 7.35302 18.3195 7.41902 18.3075C7.49402 18.294 7.56902 18.288 7.64252 18.288C8.23052 18.288 8.75252 18.705 8.86202 19.302L9.31502 21.78C9.36002 22.026 9.55052 22.221 9.79652 22.2735C10.5165 22.4265 11.2545 22.503 12.0015 22.503C12.7485 22.503 13.4865 22.425 14.205 22.2735C14.451 22.221 14.6415 22.026 14.6865 21.78L15.141 19.302C15.153 19.236 15.171 19.1715 15.1935 19.1085C15.375 18.603 15.852 18.288 16.362 18.288C16.5 18.288 16.641 18.312 16.779 18.36L19.158 19.2075C19.227 19.2315 19.2975 19.2435 19.3665 19.2435C19.539 19.2435 19.707 19.1715 19.827 19.0395C20.823 17.9415 21.5775 16.647 22.035 15.2355C22.113 14.997 22.038 14.736 21.846 14.574L21.8475 14.5725ZM19.092 17.589L17.2815 16.944C16.9845 16.839 16.6755 16.785 16.362 16.785C15.2085 16.785 14.1705 17.514 13.782 18.5985C13.731 18.738 13.6935 18.882 13.6665 19.029L13.3215 20.9055C12.8865 20.9685 12.444 21 12.0015 21C11.559 21 11.1165 20.9685 10.68 20.904L10.3365 19.0275C10.098 17.727 8.96552 16.7835 7.64252 16.7835C7.48052 16.7835 7.31552 16.7985 7.14902 16.8285C7.00352 16.8555 6.86102 16.893 6.72002 16.9425L4.90952 17.5875C4.35752 16.896 3.91652 16.1385 3.59102 15.321L5.05202 14.0865C5.61152 13.614 5.95202 12.951 6.01202 12.222C6.07202 11.493 5.84252 10.785 5.36702 10.227C5.27102 10.1145 5.16452 10.008 5.05202 9.912L3.59102 8.6775C3.91652 7.86 4.35752 7.101 4.90952 6.411L6.72002 7.056C7.01702 7.161 7.32602 7.215 7.64102 7.215C8.79452 7.215 9.83252 6.486 10.221 5.4015C10.272 5.2605 10.3095 5.1165 10.3365 4.971L10.68 3.0945C11.1165 3.0315 11.559 2.9985 12.0015 2.9985C12.444 2.9985 12.8865 3.03 13.3215 3.093L13.665 4.9695C13.9035 6.27 15.036 7.2135 16.359 7.2135C16.521 7.2135 16.686 7.1985 16.851 7.1685C16.9965 7.1415 17.1405 7.104 17.2815 7.0545L19.092 6.4095C19.644 7.0995 20.085 7.8585 20.4105 8.676L18.951 9.9105C18.3915 10.383 18.0495 11.046 17.991 11.775C17.931 12.504 18.1605 13.2135 18.636 13.77C18.7335 13.884 18.8385 13.989 18.9525 14.085L20.4135 15.3195C20.088 16.137 19.647 16.896 19.095 17.586L19.092 17.589Z" bg=current }}
icon gear-sm viewbox=24 {{ path "M12 8 A4 4 0 1 0 12 16 A4 4 0 1 0 12 8 M12 2 L12 5 M12 19 L12 22 M2 12 L5 12 M19 12 L22 12 M4.5 4.5 L6.6 6.6 M17.4 17.4 L19.5 19.5 M19.5 4.5 L17.4 6.6 M6.6 17.4 L4.5 19.5" bg=none stroke=current stroke-w=2.8 }}
icon lines viewbox=24 {{ path "M4 7 L20 7 M4 12 L20 12 M4 17 L20 17" bg=none stroke=current stroke-w=3 }}
icon zap viewbox=24 {{ path "M13 2 L5 14 L11 14 L9 22 L19 9 L12.5 9 Z" bg=none stroke=current stroke-w=2.6 }}
icon bell viewbox=24 {{ path "M6 17 L6 11 C6 7 8 5 12 5 C16 5 18 7 18 11 L18 17 Z M10 19 C10 21 14 21 14 19" bg=none stroke=current stroke-w=2.6 }}
icon refresh viewbox=24 {{ path "M19 12 A7 7 0 1 1 12 5 M12 5 L16 3 M12 5 L14 9" bg=none stroke=current stroke-w=3 }}
icon grid4 viewbox=24 {{ path "M4 4 L20 4 L20 20 L4 20 Z M12 4 L12 20 M4 12 L20 12" bg=none stroke=current stroke-w=3 }}
icon mic viewbox=24 {{ path "M9 5 C9 2 15 2 15 5 L15 12 C15 15 9 15 9 12 Z M6 12 C6 19 18 19 18 12 M12 18 L12 22" bg=none stroke=current stroke-w=3.2 }}
icon enter viewbox=24 {{ path "M20 5 L20 13 L6 13 M6 13 L10 9 M6 13 L10 17" bg=none stroke=current stroke-w=3.2 }}
icon swap viewbox=24 {{ path "M4 8 L18 8 M18 8 L14 4 M6 16 L20 16 M6 16 L10 20" bg=none stroke=current stroke-w=3.2 }}
icon expand viewbox=24 {{ path "M14 4 L20 4 L20 10 M20 4 L13 11 M10 20 L4 20 L4 14 M4 20 L11 13" bg=none stroke=current stroke-w=3 }}
icon circ viewbox=24 {{ path "M12 4 A8 8 0 1 0 12 20 A8 8 0 1 0 12 4 M12 9 A3 3 0 1 0 12 15 A3 3 0 1 0 12 9" bg=none stroke=current stroke-w=3.6 }}
icon square2 viewbox=24 {{ path "M8 4 L20 4 L20 16 L8 16 Z M4 8 L4 20 L16 20" bg=none stroke=current stroke-w=3.4 }}
icon folder viewbox=24 {{ path "M3 6 L9 6 L11 8 L21 8 L21 19 L3 19 Z" bg=none stroke=current stroke-w=2.2 }}
icon folder-open viewbox=24 {{ path "M3 6 L9 6 L11 8 L21 8 L21 11 M3 19 L5.5 11 L23 11 L20 19 Z M3 6 L3 19" bg=none stroke=current stroke-w=2.2 }}
icon scm viewbox=24 {{ path "M21 8.25C21 6.1815 19.3185 4.5 17.25 4.5C15.1815 4.5 13.5 6.1815 13.5 8.25C13.5 10.023 14.739 11.5035 16.395 11.892C16.116 12.819 15.2655 13.5 14.25 13.5H9.75C8.9025 13.5 8.1285 13.7925 7.5 14.268V7.4235C9.21 7.0755 10.5 5.5605 10.5 3.75C10.5 1.6815 8.8185 0 6.75 0C4.6815 0 3 1.6815 3 3.75C3 5.562 4.29 7.0755 6 7.4235V16.575C4.29 16.923 3 18.438 3 20.2485C3 22.317 4.6815 23.9985 6.75 23.9985C8.8185 23.9985 10.5 22.317 10.5 20.2485C10.5 18.4755 9.261 16.995 7.605 16.6065C7.884 15.6795 8.7345 14.9985 9.75 14.9985H14.25C16.0845 14.9985 17.61 13.6725 17.931 11.9295C19.674 11.607 21 10.0845 21 8.25ZM4.5 3.75C4.5 2.5095 5.5095 1.5 6.75 1.5C7.9905 1.5 9 2.5095 9 3.75C9 4.9905 7.9905 6 6.75 6C5.5095 6 4.5 4.9905 4.5 3.75ZM9 20.25C9 21.4905 7.9905 22.5 6.75 22.5C5.5095 22.5 4.5 21.4905 4.5 20.25C4.5 19.0095 5.5095 18 6.75 18C7.9905 18 9 19.0095 9 20.25ZM17.25 10.5C16.0095 10.5 15 9.4905 15 8.25C15 7.0095 16.0095 6 17.25 6C18.4905 6 19.5 7.0095 19.5 8.25C19.5 9.4905 18.4905 10.5 17.25 10.5Z" bg=current }}
icon debug-alt viewbox=24 {{ path "M19.854 13.9605L13.2105 17.697C12.954 17.22 12.5505 16.8345 12.039 16.641L12.054 16.626L19.1175 12.6525C19.6275 12.366 19.6275 11.6325 19.1175 11.3445L7.11751 4.59599C6.61801 4.31399 6.00001 4.67549 6.00001 5.24999V10.5C5.46901 10.5 4.97401 10.6215 4.50001 10.791V5.24999C4.50001 3.52949 6.35251 2.44499 7.85251 3.28949L19.8525 10.0395C21.381 10.899 21.381 13.101 19.8525 13.962L19.854 13.9605ZM10.5 16.0605V18H11.25C11.664 18 12 18.336 12 18.75C12 19.164 11.664 19.5 11.25 19.5H10.5C10.5 20.076 10.3905 20.625 10.1925 21.132L11.781 22.7205C12.0735 23.013 12.0735 23.4885 11.781 23.781C11.634 23.928 11.442 24 11.25 24C11.058 24 10.866 23.9265 10.719 23.781L9.39151 22.4535C8.56651 23.4 7.35151 24.0015 6.00001 24.0015C4.64851 24.0015 3.43351 23.4015 2.60851 22.4535L1.28101 23.781C1.13401 23.928 0.942009 24 0.750009 24C0.558009 24 0.366009 23.9265 0.219009 23.781C-0.0734912 23.4885 -0.0734912 23.013 0.219009 22.7205L1.80751 21.132C1.60951 20.625 1.50001 20.076 1.50001 19.5H0.750009C0.336009 19.5 8.78423e-06 19.164 8.78423e-06 18.75C8.78423e-06 18.336 0.336009 18 0.750009 18H1.50001V16.0605L0.219009 14.7795C-0.0734912 14.487 -0.0734912 14.0115 0.219009 13.719C0.511509 13.4265 0.987009 13.4265 1.27951 13.719L2.56051 15H3.00001C3.00001 13.3455 4.34551 12 6.00001 12C7.65451 12 9.00001 13.3455 9.00001 15H9.43951L10.7205 13.719C11.013 13.4265 11.4885 13.4265 11.781 13.719C12.0735 14.0115 12.0735 14.487 11.781 14.7795L10.5 16.0605ZM4.50001 15H7.50001C7.50001 14.172 6.82801 13.5 6.00001 13.5C5.17201 13.5 4.50001 14.172 4.50001 15ZM9.00001 16.5H3.00001V19.5C3.00001 21.1545 4.34551 22.5 6.00001 22.5C7.65451 22.5 9.00001 21.1545 9.00001 19.5V16.5Z" bg=current }}
icon extensions viewbox=16 {{ path "M15 4.95703C15 4.58711 14.8563 4.24054 14.5949 3.97992L12.0096 1.39234C11.4879 0.86922 10.5788 0.86922 10.0571 1.39234L8 3.45119V3.32321C8 2.55068 7.37187 1.922 6.6 1.922H2.4C1.62813 1.922 1 2.55068 1 3.32321V13.5988C1 14.3713 1.62813 15 2.4 15H12.6667C13.4385 15 14.0667 14.3713 14.0667 13.5988V9.39514C14.0667 8.62261 13.4385 7.99393 12.6667 7.99393H12.5379L14.5949 5.93508C14.8553 5.67445 15 5.32602 15 4.95703ZM2.4 2.85521H6.6C6.85667 2.85521 7.06667 3.06446 7.06667 3.32228V7.99299H1.93333V3.32228C1.93333 3.06446 2.14333 2.85521 2.4 2.85521ZM1.93333 13.5979V8.92714H7.06667V14.0649H2.4C2.14333 14.0649 1.93333 13.8547 1.93333 13.5979ZM13.1333 9.39421V13.5979C13.1333 13.8547 12.9233 14.0649 12.6667 14.0649H8V8.92714H12.6667C12.9233 8.92714 13.1333 9.13638 13.1333 9.39421ZM8 7.99299V6.46287L9.5288 7.99299H8ZM13.9351 5.2737L11.3488 7.86221C11.1789 8.03223 10.8859 8.03223 10.716 7.86221L8.12973 5.2737C8.0448 5.18963 7.99813 5.07753 7.99813 4.95796C7.99813 4.83839 8.0448 4.7263 8.12973 4.64129L10.716 2.05278C10.8009 1.96777 10.9129 1.92106 11.0324 1.92106C11.1519 1.92106 11.2639 1.96777 11.3488 2.05278L13.9351 4.64129C14.02 4.72536 14.0667 4.83746 14.0667 4.95703C14.0667 5.0766 14.02 5.1887 13.9351 5.2737Z" bg=current }}
icon remote viewbox=25 {{ path "M9.32 20.0677C9.469 20.5907 9.667 21.0917 9.911 21.5677H3.759C3.345 21.5677 3.009 21.2317 3.009 20.8177C3.009 20.4037 3.345 20.0677 3.759 20.0677H6.008V18.5517H3C1.343 18.5517 0 17.2087 0 15.5517V5.06775C0 3.41075 1.343 2.06775 3 2.06775H16.5C18.157 2.06775 19.5 3.41075 19.5 5.06775V9.88775C19.016 9.74975 18.516 9.65275 18 9.60575V5.06775C18 4.23975 17.328 3.56775 16.5 3.56775H3C2.172 3.56775 1.5 4.23975 1.5 5.06775V15.5517C1.5 16.3797 2.172 17.0517 3 17.0517H9.039C9.016 17.3047 9 17.5587 9 17.8177C9 18.0657 9.016 18.3097 9.037 18.5517H7.507V20.0677H9.32ZM24 17.8177C24 21.5457 20.978 24.5677 17.25 24.5677C13.522 24.5677 10.5 21.5457 10.5 17.8177C10.5 14.0897 13.522 11.0677 17.25 11.0677C20.978 11.0677 24 14.0897 24 17.8177ZM17.251 19.3177C17.251 19.2187 17.231 19.1217 17.194 19.0307C17.156 18.9397 17.101 18.8567 17.031 18.7867L14.781 16.5367C14.64 16.3957 14.449 16.3167 14.25 16.3167C14.051 16.3167 13.86 16.3957 13.719 16.5367C13.578 16.6777 13.499 16.8687 13.499 17.0677C13.499 17.2667 13.578 17.4577 13.719 17.5987L15.44 19.3177L13.719 21.0367C13.578 21.1777 13.499 21.3687 13.499 21.5677C13.499 21.7667 13.578 21.9577 13.719 22.0987C13.86 22.2397 14.051 22.3187 14.25 22.3187C14.449 22.3187 14.64 22.2397 14.781 22.0987L17.031 19.8487C17.101 19.7787 17.156 19.6967 17.194 19.6057C17.232 19.5147 17.251 19.4167 17.251 19.3177ZM19.06 16.3177L20.78 14.5987C20.921 14.4577 21 14.2667 21 14.0677C21 13.8687 20.921 13.6777 20.78 13.5367C20.639 13.3957 20.448 13.3167 20.249 13.3167C20.05 13.3167 19.859 13.3957 19.718 13.5367L17.468 15.7867C17.398 15.8567 17.343 15.9387 17.305 16.0307C17.267 16.1217 17.248 16.2197 17.248 16.3177C17.248 16.4157 17.268 16.5137 17.305 16.6057C17.343 16.6967 17.398 16.7797 17.468 16.8487L19.718 19.0987C19.859 19.2397 20.05 19.3187 20.249 19.3187C20.448 19.3187 20.639 19.2397 20.78 19.0987C20.921 18.9577 21 18.7667 21 18.5677C21 18.3687 20.921 18.1777 20.78 18.0367L19.06 16.3177Z" bg=current }}
icon account viewbox=16 {{ path "M6 5C6 3.89543 6.89543 3 8 3C9.10457 3 10 3.89543 10 5C10 6.10457 9.10457 7 8 7C6.89543 7 6 6.10457 6 5ZM5.49998 8L10.5 8C11.3284 8 12 8.67157 12 9.5C12 10.6161 11.541 11.5103 10.7879 12.1148C10.0466 12.7098 9.05308 13 8 13C6.94692 13 5.95342 12.7098 5.21215 12.1148C4.45897 11.5103 4 10.6161 4 9.5C4 8.67161 4.67156 8 5.49998 8ZM8 0C3.58172 0 0 3.58172 0 8C0 12.4183 3.58172 16 8 16C12.4183 16 16 12.4183 16 8C16 3.58172 12.4183 0 8 0ZM1 8C1 4.13401 4.13401 1 8 1C11.866 1 15 4.13401 15 8C15 11.866 11.866 15 8 15C4.13401 15 1 11.866 1 8Z" bg=current }}
icon markdown viewbox=24 {{ path "M3 5 L21 5 L21 19 L3 19 Z M6 15 L6 9 L9 12 L12 9 L12 15 M15 11 L18 14 L21 11 M18 9 L18 14" bg=none stroke=current stroke-w=2.2 }}
icon shellfile viewbox=24 {{ path "M4 4 L16 4 L20 8 L20 20 L4 20 Z M16 4 L16 8 L20 8 M7 11 L10 14 L7 17 M12 17 L17 17" bg=none stroke=current stroke-w=2.2 }}
icon cmakefile viewbox=24 {{ path "M12 3 L22 20 L2 20 Z M12 3 L12 14 M2 20 L12 14 L22 20" bg=none stroke=current stroke-w=2.2 }}
icon gearfile viewbox=24 {{ path "M5 3 L15 3 L19 7 L19 12 M15 3 L15 7 L19 7 M12 11 A3 3 0 1 0 12 17 A3 3 0 1 0 12 11 M12 9 L12 11 M12 17 L12 19 M7 14 L9 14 M15 14 L17 14 M8.5 10.5 L9.8 11.8 M14.2 16.2 L15.5 17.5 M15.5 10.5 L14.2 11.8 M9.8 16.2 L8.5 17.5" bg=none stroke=current stroke-w=2.2 }}
''')

A(MENU_ROW_DEF)
A(QO_ROW_DEF)
A(EDITOR_TAB_DEF)
A(TREE_ROW_DEF)
A('''def ChangeRow(file="", badge="M") export {
  row h=22 gap=6 pad=0,16 align=center act=scm_pick label="change" {
    when hover { bg=#FFFFFF08 }
    text file size=12 color=#B6B7BE nowrap ellipsis w=fill
    text badge size=11 weight=700 color=#D9A33C nowrap
  }
}

def SessionRow(title="") export {
  row h=22 gap=6 pad=0,10 align=center {
    icon chev-r size=8 color=#55555A
    text title size=12 color=#B6B7BE nowrap ellipsis
  }
}

params scm {
  changes list(ChangeRow) = []
}

params chat {
  sessions list(SessionRow) = []
  typing bool = false
}''')
A('''def SearchRow(file="", line=0, preview="") export {
  col pad=2,16 gap=1 act=search_pick label="search result" {
    when hover { bg=#FFFFFF08 }
    row gap=6 {
      text file size=12 color=#909094 nowrap
      text line size=11 color=#55555A nowrap
    }
    text preview size=12 color=#B6B7BE nowrap ellipsis
  }
}''')
A('''def ProblemRow(icon="errc", file="", line="", msg="", tint="red") export {
  row h=22 gap=6 align=center act=problem_pick {
    when hover { bg=#FFFFFF08 }
    icon icon size=16 color=tint
    text msg size=13 color=#B6B7BE nowrap
  }
}

params problems {
  filtering bool = false
  rows list(ProblemRow) = [
    ProblemRow(icon="errc", file="include/agentfs/mdb.hpp", line="47", msg="mdb.hpp(47,18): use of undeclared identifier 'MDB_CREATE'", tint="#A63D5B"),
    ProblemRow(icon="errc", file="include/agentfs/mdb.hpp", line="68", msg="mdb.hpp(68,11): no matching function for call to 'mdb_txn_begin'", tint="#A63D5B"),
    ProblemRow(icon="errc", file="include/agentfs/store.hpp", line="92", msg="store.hpp(92,7): unknown type name 'MDB_cursor'", tint="#A63D5B"),
    ProblemRow(icon="errc", file="include/agentfs/store.hpp", line="118", msg="store.hpp(118,24): member reference base type is not a structure or union", tint="#A63D5B"),
    ProblemRow(icon="warn", file="include/agentfs/mdb.hpp", line="31", msg="mdb.hpp(31,9): unused variable 'flags'", tint="#D9A33C"),
    ProblemRow(icon="warn", file="include/agentfs/store.hpp", line="143", msg="store.hpp(143,16): implicit conversion changes signedness", tint="#D9A33C")
  ]
}''')
A('''def ExtRow(name="", publisher="") export {
  row h=40 gap=8 pad=0,10 align=center {
    rect w=28 h=28 radius=4 bg=#2D6CB3
    col gap=2 {
      text name size=13 color=#C9C9CC nowrap
      text publisher size=11 color=#707074 nowrap ellipsis
    }
  }
}''')
A('''params sidebar {
  explorer bool = true
  search bool = false
  scm bool = false
  debug bool = false
  ext bool = false
}

params search {
  results list(SearchRow) = []
}''')
A('''params ext {
  rows list(ExtRow) = [
    ExtRow(name="C/C++", publisher="ms-vscode.cpptools"),
    ExtRow(name="CMake Tools", publisher="ms-vscode.cmake-tools"),
    ExtRow(name="GitLens", publisher="eamodio.gitlens")
  ]
}''')


# root
A('stack w=fill h=fill {')
A(f'  col w=fill h=fill min-w=980 min-h=560 bg=color.bg gap=0 family="Inter" {{')

# ---- title bar
A(f'''  row h=28 align=center bg=color.bg {{
    rect w=12 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlRed']}
    rect w=4 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlYel']}
    rect w=4 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlGrn']}
    spacer
    col w=20 h=20 align=center pack=center act=nav_back label="Back" color=color.uiFaint {{
      when nav.canback {{ color=color.ui }}
      when hover {{ bg=#FFFFFF10 radius=4 }}
      text "←" size=13 nowrap
    }}
    rect w=14 h=1 bg=none
    col w=20 h=20 align=center pack=center act=nav_fwd label="Forward" color=color.uiFaint {{
      when nav.canfwd {{ color=color.ui }}
      when hover {{ bg=#FFFFFF10 radius=4 }}
      text "→" size=13 nowrap
    }}
    rect w=14 h=1 bg=none
    row w=330 h=22 radius=6 bg=#171719 align=center pad=0,10 {{
      text "agentfs-cxx" size=13 color=color.ui nowrap
      spacer
      icon refresh size=12 color=color.uiDim
      rect w=6 h=1 bg=none
      text "⌄" size=10 color=color.uiFaint nowrap
    }}
    rect w=10 h=1 bg=none
    rect w=15 h=15 radius=999 stroke=color.uiDim stroke-w=1.5 {{ }}
    spacer
    icon grid4 size=12 color=color.uiDim
    rect w=12 h=1 bg=none
    rect w=14 h=11 radius=2 stroke=color.uiDim stroke-w=1.2 {{ rect w=5 h=9 bg=color.ui offset=0.8,0.8 }}
    rect w=12 h=1 bg=none
    rect w=14 h=11 radius=2 stroke=color.uiDim stroke-w=1.2 {{ rect w=12 h=4 bg=color.ui offset=0.8,5.5 }}
    rect w=12 h=1 bg=none
    rect w=14 h=11 radius=2 stroke=color.uiDim stroke-w=1.2 {{ rect w=5 h=9 bg=color.ui offset=8,0.8 }}
    rect w=12 h=1 bg=none
  }}''')

# ---- main row
A('  row h=fill gap=0 {')

# activity bar
A(f'''    col w=55 bg=color.bg2 align=center pad=8,0 gap=0 {{
      col h=44 w=fill align=center pack=center act=activity_pick key=explorer label="Explorer" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon files size=22
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=search label="Search" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon search size=22
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=scm label="Source Control" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon scm size=22
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=debug label="Run and Debug" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon debug-alt size=22
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=ext label="Extensions" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        stack w=24 h=24 {{
          icon extensions size=22
          rect w=9 h=9 radius=999 bg={C['yellow']} self=bottom-end
        }}
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=remote label="Remote Explorer" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon remote size=22
      }}
      spacer
      col h=44 w=fill align=center pack=center act=activity_pick key=account label="Accounts" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon account size=22
      }}
      col h=44 w=fill align=center pack=center act=activity_pick key=settings label="Manage" color=color.uiFaint {{
        when selected {{ color=#E7E7E9 stroke=#C9C9CC stroke-w=2 stroke-sides=l }}
        when hover {{ color=#C9C9CC }}
        icon gear size=22
      }}
    }}''')

# sidebar
A(f'''    col #sidebar w=fill:195 min-w=0 max-w=560 bg=color.bg2 gap=0 clip {{
     stack #views w=fill h=fill {{
      col #view-explorer w=fill h=fill gap=0 clip {{
        when !sidebar.explorer {{ max-w=0 max-h=0 }}
        row h=32 align=center pad=0,16 {{
          text "EXPLORER" size=11 color=color.uiDim tracking=0.4 nowrap
          spacer
          text "···" size=12 color=color.uiDim nowrap
        }}
        row h=22 align=center pad=0,6 gap=3 {{
          icon chev-d size=10 color=color.uiDim
          text "AGENTFS-CXX" size=11 weight=700 color=color.ui tracking=0.3 nowrap
        }}
        col #treescroll w=fill h=fill scroll clip {{''')
A('        each param.tree #treerows')
A(f'''        }}
        row h=22 align=center pad=0,6 gap=3 {{ icon chev-r size=10 color=color.uiFaint; text "OUTLINE" size=11 weight=700 color=color.uiDim tracking=0.3 nowrap }}
        row h=22 align=center pad=0,6 gap=3 {{ icon chev-r size=10 color=color.uiFaint; text "TIMELINE" size=11 weight=700 color=color.uiDim tracking=0.3 nowrap }}
        rect h=12 bg=none
      }}
      col #view-search w=fill h=fill gap=0 scroll clip {{
        when !sidebar.search {{ max-w=0 max-h=0 }}
        row h=32 align=center pad=0,16 {{
          text "SEARCH" size=11 color=color.uiDim tracking=0.4 nowrap
          spacer
          text "···" size=12 color=color.uiDim nowrap
        }}
        row h=24 bg=#1B1B24 radius=3 pad=0,6 {{
          text "" field=sidebar_search_change w=fill size=13 color=#D5D5DA nowrap label="Search files"
        }}
        text "No results yet." size=12 color=#55555A pad-l=16 nowrap
        each param.search.results #searchrows
      }}
      col #view-scm w=fill h=fill gap=0 scroll clip {{
        when !sidebar.scm {{ max-w=0 max-h=0 }}
        row h=32 align=center pad=0,16 {{
          text "SOURCE CONTROL" size=11 color=color.uiDim tracking=0.4 nowrap
          spacer
          text "···" size=12 color=color.uiDim nowrap
        }}
        row h=24 align=center gap=6 pad=0,16 {{
          icon branch size=14 color=color.uiDim
          text "main" size=12 color=color.ui nowrap
        }}
        text "0 pending changes" size=12 color=#55555A pad-l=16 nowrap
        each param.scm.changes #scmrows
      }}
      col #view-debug w=fill h=fill gap=0 scroll clip {{
        when !sidebar.debug {{ max-w=0 max-h=0 }}
        row h=32 align=center pad=0,16 {{
          text "RUN AND DEBUG" size=11 color=color.uiDim tracking=0.4 nowrap
          spacer
          text "···" size=12 color=color.uiDim nowrap
        }}
        row h=26 bg=#2D6CB3 radius=4 align=center pack=center {{
          text "Run and Debug" size=13 color=#FFFFFF nowrap
        }}
        text "To customize Run and Debug create a launch.json file." w=fill size=11 color=#55555A pad=8,16
      }}
      col #view-ext w=fill h=fill gap=0 scroll clip {{
        when !sidebar.ext {{ max-w=0 max-h=0 }}
        row h=32 align=center pad=0,16 {{
          text "EXTENSIONS" size=11 color=color.uiDim tracking=0.4 nowrap
          spacer
          text "···" size=12 color=color.uiDim nowrap
        }}
        row h=24 bg=#1B1B24 radius=3 pad=0,6 {{
          text "" field=ext_search_change w=fill size=13 color=#D5D5DA nowrap label="Search extensions"
        }}
        each param.ext.rows #extrows
      }}
     }}
    }}''')

# sidebar sash, then editors + panel column
A(f'''    divider #sashL w=4 bg=none label="Resize sidebar" resize=sash_sidebar {{
      when hover {{ bg=#3C7BACAA }}
      when pressed {{ bg=#3C7BACDD }}
    }}''')
A('    col #center w=fill:1113 min-w=420 gap=0 {')
A('      row #grid h=fill:613 min-h=200 gap=0 {')

# editor grid (recursive groups; kernel splits container owns sashes)
A('        each param.root #egroups')
A('      }')  # end editor row

# ---- bottom panel
A(f'''      divider #sashP h=4 bg=none label="Resize panel" stroke=color.sep stroke-w=0.5 stroke-sides=t resize=sash_panel {{
        when hover {{ bg=#3C7BACAA }}
        when pressed {{ bg=#3C7BACDD }}
      }}''')
A(f'''      col #panel h=fill:178 min-h=77 bg=color.bg gap=0 clip {{
        row h=30 align=center pad=0,14 gap=16 {{
          row gap=5 align=center h=fill act=panel_pick key=problems color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "PROBLEMS" size=11 nowrap
            row w=16 h=16 radius=999 bg={C['red']} align=center pack=center {{ text "6" size=10 color=#FFFFFF nowrap }}
          }}
          col h=fill pack=center act=panel_pick key=output color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "OUTPUT" size=11 nowrap
          }}
          col h=fill pack=center act=panel_pick key=debugc color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "DEBUG CONSOLE" size=11 nowrap
          }}
          col h=fill pack=center act=panel_pick key=terminal color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "TERMINAL" size=11 nowrap
          }}
          col h=fill pack=center act=panel_pick key=ports color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "PORTS" size=11 nowrap
          }}
          spacer
          row w=200 h=22 radius=3 bg=#1A191E align=center pad=0,7 {{
            stack w=fill {{
              text "Filter (e.g. text, !excludeTex..." size=12 color=color.uiFaint nowrap inert {{ when problems.filtering {{ opacity=0 }} }}
              text "" field=problems_filter_change w=fill size=12 color=#C9C9CC nowrap label="Filter problems"
            }}
          }}
          row gap=4 align=center {{
            text "C/C++ Configuration Warn" size=12 color=color.ui nowrap
            text "⌄" size=10 color=color.uiFaint nowrap
          }}
          icon lines size=12 color=color.uiDim
          icon expand size=12 color=color.uiDim act=panel_max label="Maximize panel"
          icon close size=10 color=color.uiDim act=panel_close label="Close panel"
        }}
        stack #panelviews w=fill h=fill {{
        col #panel-problems w=fill h=fill pad=4,14 gap=2 scroll clip {{
          when !panel.problems {{ max-w=0 max-h=0 }}
          each param.problems.rows #problemrows
        }}
        col #panel-output w=fill h=fill scroll clip {{
          when !panel.output {{ max-w=0 max-h=0 }}
          col w=fill h=fill pad=4,14 gap=0 family="JetBrains Mono" leading=1.5 select select-bg=#264F7866 {{
            para {{ span "[7/30/2026, 5:08:54 PM] For C source files, IntelliSenseMode was changed from \\"macos-clang-x64\\" to \\"macos-clang-arm64\\" based on compiler args and querying compilerPath: \\"/usr/bin/clang\\"" color=#BEBDC0 size=12 }}
            para {{ span "[7/30/2026, 5:08:54 PM] IntelliSenseMode was changed because it didn't match the detected compiler.  Consider setting \\"compilerPath\\" instead.  Set \\"compilerPath\\" to \\"\\" to disable detection of system includes and defines." color=#BEBDC0 size=12 }}
            para {{ span "[7/30/2026, 5:08:54 PM] For C++ source files, IntelliSenseMode was changed from \\"macos-clang-x64\\" to \\"macos-clang-arm64\\" based on compiler args and querying compilerPath: \\"/usr/bin/clang\\"" color=#BEBDC0 size=12 }}
            para {{ span "[7/30/2026, 5:08:54 PM] IntelliSenseMode was changed because it didn't match the detected compiler.  Consider setting \\"compilerPath\\" instead.  Set \\"compilerPath\\" to \\"\\" to disable detection of system includes and defines." color=#BEBDC0 size=12 }}
          }}
        }}
        col #panel-debugc w=fill h=fill scroll clip pad=4,14 gap=2 {{
          when !panel.debugc {{ max-w=0 max-h=0 }}
          text "Debug console is only available during a debug session." size=12 color=#55555A
        }}
        col #panel-terminal w=fill h=fill scroll clip pad=4,14 gap=2 family="JetBrains Mono" {{
          when !panel.terminal {{ max-w=0 max-h=0 }}
          text param.panel.termlog size=12 color=#B6B7BE family="JetBrains Mono"
          row gap=6 {{ text "can@mac slab-lang %" size=12 color=#3FB950 nowrap; text "" field=term_change submit=term_send size=12 color=#D5D5DA nowrap w=fill label="Terminal input" }}
        }}
        col #panel-ports w=fill h=fill scroll clip pad=4,14 gap=2 {{
          when !panel.ports {{ max-w=0 max-h=0 }}
          text "No forwarded ports. Forward a port to access your services over the internet." size=12 color=#55555A
        }}
        }}
      }}
    }}''')  # end editors+panel col

# ---- chat panel
A(f'''    divider #sashR w=4 bg=none label="Resize chat" stroke=color.sep stroke-w=0.5 stroke-sides=l resize=sash_center {{
      when hover {{ bg=#3C7BACAA }}
      when pressed {{ bg=#3C7BACDD }}
    }}
    col #chat w=fill:198 min-w=150 max-w=640 bg=color.bg2 gap=0 clip {{
      row h=28 align=center pad=0,10 gap=8 {{
        text "CHAT" size=11 weight=600 color=#C6C0C4 tracking=0.3 nowrap
        spacer
        text "+" size=14 color=color.uiDim nowrap
        icon gear-sm size=12 color=color.uiDim
        text "···" size=12 color=color.uiDim nowrap
        icon expand size=11 color=color.uiDim
        icon close size=10 color=color.uiDim
      }}
      row h=24 align=center pad=0,10 {{
        text "SESSIONS" size=11 color=color.uiDim tracking=0.4 nowrap
        spacer
        icon search-sm size=11 color=color.uiFaint
        rect w=8 h=1 bg=none
        icon lines size=11 color=color.uiFaint
      }}
      each param.chat.sessions #sessionrows
      spacer
      col pad=0,10 gap=3 {{
        para {{
          span "Tip: " color=#B9B9BC size=12
          span "Try the " color=color.uiDim size=12
          span "Plan agent" color={C['red']} size=12
          span " to research and plan before implementing changes." color=color.uiDim size=12
        }}
      }}
      rect h=8 bg=none
      col pad=0,8 gap=0 {{
        col radius=8 bg={C['cardBg']} stroke=#232228 stroke-w=1 pad=8 gap=7 {{
          row gap=4 align=center {{
            text "+" size=12 color=color.uiDim nowrap
            text "h" size=10 weight=700 color=color.blue nowrap family="JetBrains Mono"
            text "store.hpp" size=11 color=color.uiDim italic nowrap
          }}
          stack w=fill {{
            text "Describe what to build" size=13 color=#55555A nowrap inert {{ when !chat.typing {{ opacity=1 }} when chat.typing {{ opacity=0 }} }}
            text "" field=chat_change submit=chat_send w=fill size=13 color=#C9C9CC nowrap
          }}
          row gap=7 align=center {{
            text "+" size=13 color=color.uiDim nowrap
            text "∞" size=12 color=color.uiDim nowrap
            text "Agent" size=12 color=#B9B9BC nowrap
            icon circ size=10 color=color.uiDim
            text "Auto" size=12 color=#B9B9BC nowrap
            icon swap size=11 color=color.uiDim
            spacer
            icon mic size=11 color=color.uiDim
            icon enter size=11 color=color.uiDim
          }}
        }}
      }}
      row h=28 align=center pad=0,12 gap=5 {{
        icon square2 size=10 color=color.uiDim
        text "Local" size=12 color=color.uiDim nowrap
        rect w=8 h=1 bg=none
        icon circ size=10 color=color.uiDim
        text "Default approvals" size=12 color=color.uiDim nowrap
      }}
    }}''')

A('  }')  # end main row

# ---- status bar
A(f'''  row h=22 align=center bg=color.bg pad=0,10 gap=0 {{
    icon zap size=14 color=color.ui
    rect w=14 h=1 bg=none
    row h=fill align=center act=status_problems label="Show Problems" {{
      icon errc size=14 color=color.uiDim
      rect w=3 h=1 bg=none
      text param.status.errs size=12 color=color.uiDim nowrap
      rect w=6 h=1 bg=none
      icon warn size=14 color=color.uiDim
      rect w=3 h=1 bg=none
      text param.status.warns size=12 color=color.uiDim nowrap
    }}
    rect w=14 h=1 bg=none
    icon gear-sm size=13 color=color.uiDim
    rect w=4 h=1 bg=none
    text "Build" size=12 color=color.uiDim nowrap
    rect w=14 h=1 bg=none
    icon grid4 size=12 color=color.uiDim
    rect w=14 h=1 bg=none
    icon play size=13 color=color.uiDim
    rect w=8 h=1 bg=none
    text "Auto Attach: With Flag" size=12 color=color.uiDim nowrap
    spacer
    text param.status.caret size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "Tab Size: 3" size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "UTF-8" size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "LF" size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "{{}}" size=12 color=color.uiDim nowrap
    rect w=4 h=1 bg=none
    text param.status.lang size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "Mac" size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "⊘ Prettier" size=12 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    icon bell size=13 color=color.uiDim
  }}
  rect h=3 bg=#000000
}}''')
A('''  col #menu w=220 bg=#1F1E25 radius=6 stroke=#38373E stroke-w=1 shadow=0,4,16,#00000088 pad=4,0 attach=param.menu.anchor gravity=below-start collide=auto {
    when !menu.open { max-w=0 max-h=0 }
    each param.menu.items #menuitems
  }
  col #quickopen w=560 self=top offset=0,60 bg=#1F1E25 radius=8 stroke=#38373E stroke-w=1 shadow=0,8,32,#000000AA pad=6 gap=4 {
    when !qo.open { max-w=0 max-h=0 }
    row h=28 bg=#16151B radius=4 pad=0,8 align=center {
      text "" field=qo_change w=fill size=13 color=#C9C9CC nowrap label="Search files by name"
    }
    col #qorows {
      each param.qo.rows #qorowitems
    }
  }
}''')

open(__file__.replace('gen.py', 'vscode.slab'), 'w').write('\n'.join(doc) + '\n')
print('wrote vscode.slab,', len('\n'.join(doc).splitlines()), 'lines')
