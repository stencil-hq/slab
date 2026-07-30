#!/usr/bin/env python3
"""Generates vscode.slab — a 1:1 static replica of the reference Cursor/VS Code
screenshot (1568x844). Source of truth for the demo; edit here, then:
    python3 demos/vscode/gen.py && cargo run -q -p slab-cli -- render demos/vscode/vscode.slab -o /tmp/vscode.png --width 1568 --height 844
"""
import re

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
    kw        = '#9D74BE',   # keywords / preproc (italic)
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

KW = {'constexpr','explicit','operator','const','return','if','void','struct','delete',
      'default','noexcept','namespace','using','bool','nullptr','this','class','public'}
TYPES = {'uint64_t','MDB_env','MDB_txn','MDB_dbi','node_kind','dir_entry','size_t'}

def esc(s: str) -> str:
    return s.replace('\\', '\\\\').replace('"', '\\"')

TOK = re.compile(r'//.*$|"[^"]*"|<[^>\s]+>|[A-Za-z_][A-Za-z0-9_]*|\d+|.')

def hl(line: str):
    """C++ line -> [(text, color, italic)] runs, merged."""
    runs = []
    if line.lstrip().startswith('#'):  # preprocessor
        m = re.match(r'(\s*)(#\w+)(.*)', line)
        runs.append((m.group(1) + m.group(2), C['kw'], True))
        rest = m.group(3)
        for t in TOK.findall(rest):
            if t.startswith('"') or t.startswith('<'):
                runs.append((t, C['str_'], False))
            else:
                runs.append((t, C['txt'], False))
        return merge(runs)
    toks = TOK.findall(line)
    for i, t in enumerate(toks):
        if t.startswith('//'):
            runs.append((t, C['com'], True))
        elif t.startswith('"'):
            runs.append((t, C['str_'], False))
        elif t in KW:
            runs.append((t, C['kw'], True))
        elif t in TYPES:
            runs.append((t, C['typ'], False))
        elif t.isdigit():
            runs.append((t, C['typ'], False))
        elif re.match(r'[A-Za-z_]', t):
            # function call: next non-space token is '('
            j = i + 1
            while j < len(toks) and toks[j] == ' ': j += 1
            if j < len(toks) and toks[j] == '(':
                runs.append((t, C['fn'], False))
            else:
                runs.append((t, C['txt'], False))
        else:
            runs.append((t, C['txt'], False))
    return merge(runs)

def merge(runs):
    out = []
    for t, c, i in runs:
        if out and out[-1][1] == c and out[-1][2] == i:
            out[-1][0] += t
        else:
            out.append([t, c, i])
    return [(t, c, i) for t, c, i in out if t]

def para_line(runs, size=9.5):
    if not runs:
        return f'      rect w=1 h=1 bg=none'
    spans = '; '.join(
        f'span "{esc(t)}" color={c} size={size}' + (' italic' if i else '')
        for t, c, i in runs)
    return f'      para nowrap {{ {spans} }}'

def code_block(lines, first_no, active_no=None, gutter=34):
    """lines: list[str]. Emits a gutter column beside one selectable code
    column, so text selection (and copy) never touches the line numbers —
    matching VS Code, where the gutter is outside the selectable buffer."""
    nos, paras = [], []
    for k, ln in enumerate(lines):
        no = first_no + k
        nc = C['uiDim'] if no == active_no else '#444348'
        nos.append(
            f'        text "{no}" w={gutter} h=14.2 align-text=end size=9.5 color={nc} nowrap family="JetBrains Mono"')
        runs = hl(ln) if ln else []
        body = para_line(runs)
        paras.append(f'        col h=14.2 gap=0 {{\n  {body}\n        }}')
    return (
        '    row w=fill h=fill gap=12 {\n'
        '      col w=hug h=fill gap=0 {\n' + '\n'.join(nos) + '\n      }\n'
        f'      col w=fill h=fill gap=0 select select-bg=#264F7866 {{\n' + '\n'.join(paras) + '\n      }\n'
        '    }')

LEFT_CODE = [
 '   constexpr environment() = default;',
 '   constexpr environment( environment&& other ) noexcept : handle( std::exchange( other',
 '   constexpr environment& operator=( environment&& other ) noexcept',
 '   {',
 '      std::swap( handle, other.handle );',
 '      return *this;',
 '   }',
 '   environment( const environment& )            = delete;',
 '   environment& operator=( const environment& ) = delete;',
 '   ~environment() { reset(); }',
 '',
 '   void reset()',
 '   {',
 '      if ( MDB_env* previous = std::exchange( handle, nullptr ) )',
 '         mdb_env_close( previous );',
 '   }',
 '   constexpr explicit operator bool() const { return handle != nullptr; }',
 '};',
 '',
 '// Owns a transaction handle and aborts it unless it is committed first.',
 '//',
 'struct transaction',
 '{',
 '   MDB_txn* handle = nullptr;',
 '',
 '   constexpr transaction() = default;',
 '   constexpr explicit transaction( MDB_txn* handle ) : handle( handle ) {}',
 '   constexpr transaction( transaction&& other ) noexcept : handle( std::exchange( other',
 '   constexpr transaction& operator=( transaction&& other ) noexcept',
 '   {',
 '      std::swap( handle, other.handle );',
 '      return *this;',
 '   }',
 '   transaction( const transaction& )            = delete;',
 '   transaction& operator=( const transaction& ) = delete;',
 '   ~transaction() { abort(); }',
 '',
 '   void abort()',
 '   {',
 '      if ( MDB_txn* previous = std::exchange( handle, nullptr ) )',
 '         mdb_txn_abort( previous );',
]

RIGHT_CODE = [
 '#pragma once',
 '#include <memory>',
 '#include <span>',
 '#include <string>',
 '#include <string_view>',
 '#include <vector>',
 '#include <xstd/result.hpp>',
 '#include "cid.hpp"',
 '#include "mdb.hpp"',
 '#include "node.hpp"',
 '',
 'namespace agentfs',
 '{',
 '   // Wall clock nanoseconds, the unit stored in every node record.',
 '   //',
 '   uint64_t now_ns();',
 '',
 '   // One entry of a directory listing.',
 '   //',
 '   struct dir_entry',
 '   {',
 '      std::string name;',
 '      node_kind kind = node_kind::file;',
 '   };',
 '',
 '   struct reader;',
 '   struct writer;',
 '',
 '   // LMDB backed content addressed filesystem image.',
 '   //',
 '   // Three sub-databases make up the image:',
 "   //   `meta`  - format marker and chunk geometry.",
 "   //   `nodes` - normalized path -> `node_record` plus payload.",
 "   //   `blobs` - `cid` -> chunk bytes, written once per distinct chunk.",
 '   //',
 "   // Reads borrow directly from the memory map, so a `reader` must outlive",
 '   // every span it hands out.',
 '   //',
 '   struct store',
 '   {',
 '      mdb::environment env;',
 '      MDB_dbi meta   = 0;',
]

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

def tree_rows():
    out = []
    for name, depth, kind, tint, badge in TREE:
        indent = 14 + depth * 10
        color = C['red'] if tint == 'red' else (C['ui'] if kind.startswith('d') else C['uiDim'])
        chev = ''
        if kind == 'do':
            chev = f'icon chev-d size=8 color={C["uiFaint"]}'
        elif kind == 'dc':
            chev = f'icon chev-r size=8 color={C["uiFaint"]}'
        else:
            chev = 'rect w=8 h=8 bg=none'
        ic = {
          'h': f'text "h" size=10 weight=700 color={C["blue"]} nowrap family="JetBrains Mono"',
          'c': f'text "C" size=10 weight=700 color={C["blue"]} nowrap family="JetBrains Mono"',
          'm': f'icon lines size=8 color={C["uiFaint"]}',
          'do': f'rect w=0 h=0 bg=none', 'dc': f'rect w=0 h=0 bg=none',
        }[kind]
        badge_s = f'\n      spacer\n      text "{badge}" size=10 color={C["red"]} nowrap\n      rect w=10 h=1 bg=none' if badge else ''
        sel_bg = ''
        out.append(
f'''    row h=15 gap=4 align=center pad-l={indent}{sel_bg} act=tree_pick key="{esc(name)}" {{
      when hover {{ bg=#FFFFFF08 }}
      when selected {{ bg=#1D1C22 }}
      {chev}
      {ic}
      text "{esc(name)}" size=11 color={color} nowrap{badge_s}
    }}''')
    return '\n'.join(out)

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
CRUMB_MDB = """        text "include" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "agentfs" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "h" size=10 weight=700 color=color.blue nowrap family="JetBrains Mono"
        text "mdb.hpp" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "{{}}" size=10 color=color.uiFaint nowrap
        text "agentfs" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "{{}}" size=10 color=color.uiFaint nowrap
        text "mdb" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "cursor" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "cursor(MDB_cursor *)" size=10.5 color=color.uiDim nowrap"""

CRUMB_STORE = """        text "include" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "agentfs" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "h" size=10 weight=700 color=color.blue nowrap family="JetBrains Mono"
        text "store.hpp" size=10.5 color=color.uiDim nowrap
        icon chev-r size=6 color=color.uiFaint
        text "..." size=10.5 color=color.uiDim nowrap"""

MDB_STACK = f"""          rect w=1 h=fill bg=#FFFFFF0A self=top-start offset=310,0
          rect w=1 h=fill bg=#FFFFFF0A self=top-start offset=327,0
          col w=fill h=fill family="JetBrains Mono" leading=1.29 pad-t=2 {{
{code_block(LEFT_CODE, 39, gutter=48)}
          }}
          row h=22 radius=4 bg={C['findBg']} self=top-end offset=-60,2 align=center pad=0,5 gap=5 shadow=0,2,8,#00000066 {{
            icon chev-r size=6 color=color.uiDim
            row w=110 h=16 radius=3 bg=#1B1B24 align=center pad=0,5 {{
              text "robin_hood" field=find_change size=10 color=#D5D5DA nowrap
            }}
            text "Aa" size=10 color=color.uiDim nowrap
            text "ab" size=10 color=color.uiDim underline nowrap
            text ".*" size=10 color=color.uiDim nowrap
            text "No results" size=10 color={C['red']} nowrap
            text "↑" size=10 color=color.uiFaint nowrap
            text "↓" size=10 color=color.uiFaint nowrap
            icon lines size=9 color=color.uiDim
            icon close size=8 color=color.ui
          }}"""

STORE_STACK = f"""          rect w=1 h=fill bg=#FFFFFF0A self=top-start offset=63,170
          rect w=51 h=1 bg={C['red']} self=top-start offset=97,128
          col w=fill h=fill family="JetBrains Mono" leading=1.29 pad-t=2 {{
{code_block(RIGHT_CODE, 1, active_no=1, gutter=34)}
          }}
          rect w=3 h=10 bg={C['red']} self=top-end offset=-1,128"""

# Recursive editor-group grid: one def models both branch nodes (axis via
# `horizontal`, panes in `children`) and leaf groups (tab strip + breadcrumb +
# editor). The kernel `splits` container owns pane ratios and sashes; the host
# applies VS Code grid.ts semantics (insert sibling on same orientation, wrap
# on different, Sizing.Split halves the reference pane).
EGROUP_DEF = f'''def EGroup(leaf=false, horizontal=false, show_mdb=false, show_store=false, tabs=list(EditorTab), children=list(EGroup)) export {{
  stack w=fill h=fill {{
    col #panes w=fill h=fill gap=0 splits split-w=4 split-fg=#3C7BACAA resize=split_resize label="Editor groups" {{
      when horizontal {{ axis=row }}
      when leaf {{ max-w=0 max-h=0 }}
      each children #kids
    }}
    col #chrome w=fill h=fill gap=0 {{
      when !leaf {{ max-w=0 max-h=0 }}
      when leaf {{
      row #strip h=24 bg=color.bg gap=0 drop=strip_drop dblclick=strip_new label="Tab strip" {{
        each tabs #tabs
        spacer
        icon expand size=10 color=color.uiDim self=center
        rect w=10 h=1 bg=none
        icon square2 size=10 color=color.uiDim self=center
        rect w=10 h=1 bg=none
        text "···" size=11 color=color.uiDim nowrap self=center
        rect w=10 h=1 bg=none
      }}
      row #crumb h=17 bg=color.crumbBg align=center pad=0,12 gap=5 {{
        when show_mdb {{
{CRUMB_MDB}
        }}
        when show_store {{
{CRUMB_STORE}
        }}
      }}
      stack #ed w=fill h=fill bg=color.bgDark clip drop=editor_drop label="Editor" {{
        when show_mdb {{ bg=color.bg }}
        stack #content w=fill h=fill {{
          when show_mdb {{
{MDB_STACK}
          }}
          when show_store {{
{STORE_STACK}
          }}
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


EDITOR_TAB_DEF = f'''def EditorTab(name="", note="", tint={C['txt']}, badge="", active=false, hot=false) export {{
  col w=hug h=24 {{
    stack w=hug h=22 {{
      row #body h=22 gap=5 align=center pad=0,10 \\
          press=tab_press pointer-up=tab_up dblclick=tab_dbl drop=tab_drop \\
          drag=tab_drag drag-ghost drag-update=tab_move drag-end=tab_end {{
        when hover {{ bg=#FFFFFF06 }}
        text "h" size=11 weight=700 color={C['blue']} nowrap family="JetBrains Mono"
        text name size=11.5 color=tint nowrap
        text note size=9.5 color={C['uiFaint']} nowrap
        text badge size=10 color={C['red']} nowrap
        when active {{ icon close size=8 color={C['ui']} act=tab_close label="Close tab" }}
      }}
      rect #indl w=2 h=22 self=top-start bg=none inert {{ when insert-before {{ bg=#C9C9CC }} }}
      rect #indr w=2 h=22 self=top-end bg=none inert {{ when insert-after {{ bg=#C9C9CC }} }}
    }}
    rect h=2 bg=none {{ when hot {{ bg={C['rose']} }} }}
  }}
}}

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

icon chev-r viewbox=24 {{ path "M8 4 L17 12 L8 20" bg=none stroke=current stroke-w=2.5 }}
icon chev-d viewbox=24 {{ path "M4 8 L12 17 L20 8" bg=none stroke=current stroke-w=2.5 }}
icon close viewbox=24 {{ path "M5 5 L19 19 M19 5 L5 19" bg=none stroke=current stroke-w=2.2 }}
icon search viewbox=24 {{ path "M10 3 A7 7 0 1 0 10 17 A7 7 0 1 0 10 3 M15.5 15.5 L21 21" bg=none stroke=current stroke-w=2 }}
icon branch viewbox=24 {{ path "M7 6 A2.4 2.4 0 1 0 7 10.8 A2.4 2.4 0 1 0 7 6 M7 10.8 L7 16 M7 16 A2.4 2.4 0 1 0 7 20.8 A2.4 2.4 0 1 0 7 16 M17 6 A2.4 2.4 0 1 0 17 10.8 A2.4 2.4 0 1 0 17 6 M17 10.8 C17 14 7 12 7 16" bg=none stroke=current stroke-w=1.6 }}
icon files viewbox=24 {{ path "M8 4 L15 4 L15 6 L18 6 L18 20 L10 20 L10 18 L8 18 Z M10 6 L10 18 M15 6 L15 8 L18 8" bg=none stroke=current stroke-w=1.7 }}
icon play viewbox=24 {{ path "M8 5 L19 12 L8 19 Z" bg=none stroke=current stroke-w=1.8 }}
icon warn viewbox=24 {{ path "M12 4 L22 20 L2 20 Z M12 10 L12 15 M12 17.5 L12 18.5" bg=none stroke=current stroke-w=1.7 }}
icon errc viewbox=24 {{ path "M12 3 A9 9 0 1 0 12 21 A9 9 0 1 0 12 3 M6 6 L18 18" bg=none stroke=current stroke-w=1.7 }}
icon gear viewbox=24 {{ path "M12 8 A4 4 0 1 0 12 16 A4 4 0 1 0 12 8 M12 2 L12 5 M12 19 L12 22 M2 12 L5 12 M19 12 L22 12 M4.5 4.5 L6.6 6.6 M17.4 17.4 L19.5 19.5 M19.5 4.5 L17.4 6.6 M6.6 17.4 L4.5 19.5" bg=none stroke=current stroke-w=1.8 }}
icon lines viewbox=24 {{ path "M4 7 L20 7 M4 12 L20 12 M4 17 L20 17" bg=none stroke=current stroke-w=1.8 }}
icon zap viewbox=24 {{ path "M13 2 L5 14 L11 14 L9 22 L19 9 L12.5 9 Z" bg=none stroke=current stroke-w=1.6 }}
icon bell viewbox=24 {{ path "M6 17 L6 11 C6 7 8 5 12 5 C16 5 18 7 18 11 L18 17 Z M10 19 C10 21 14 21 14 19" bg=none stroke=current stroke-w=1.7 }}
icon refresh viewbox=24 {{ path "M19 12 A7 7 0 1 1 12 5 M12 5 L16 3 M12 5 L14 9" bg=none stroke=current stroke-w=1.8 }}
icon grid4 viewbox=24 {{ path "M4 4 L20 4 L20 20 L4 20 Z M12 4 L12 20 M4 12 L20 12" bg=none stroke=current stroke-w=1.6 }}
icon mic viewbox=24 {{ path "M9 5 C9 2 15 2 15 5 L15 12 C15 15 9 15 9 12 Z M6 12 C6 19 18 19 18 12 M12 18 L12 22" bg=none stroke=current stroke-w=1.6 }}
icon enter viewbox=24 {{ path "M20 5 L20 13 L6 13 M6 13 L10 9 M6 13 L10 17" bg=none stroke=current stroke-w=1.8 }}
icon swap viewbox=24 {{ path "M4 8 L18 8 M18 8 L14 4 M6 16 L20 16 M6 16 L10 20" bg=none stroke=current stroke-w=1.8 }}
icon expand viewbox=24 {{ path "M14 4 L20 4 L20 10 M20 4 L13 11 M10 20 L4 20 L4 14 M4 20 L11 13" bg=none stroke=current stroke-w=1.8 }}
icon circ viewbox=24 {{ path "M12 4 A8 8 0 1 0 12 20 A8 8 0 1 0 12 4 M12 9 A3 3 0 1 0 12 15 A3 3 0 1 0 12 9" bg=none stroke=current stroke-w=1.5 }}
icon square2 viewbox=24 {{ path "M8 4 L20 4 L20 16 L8 16 Z M4 8 L4 20 L16 20" bg=none stroke=current stroke-w=1.6 }}
''')

A(EDITOR_TAB_DEF)

# root
A(f'col w=fill h=fill min-w=980 min-h=560 bg=color.bg gap=0 family="Inter" {{')

# ---- title bar
A(f'''  row h=28 align=center bg=color.bg {{
    rect w=12 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlRed']}
    rect w=4 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlYel']}
    rect w=4 h=1 bg=none
    rect w=12 h=12 radius=999 bg={C['tlGrn']}
    spacer
    text "←" size=13 color=color.ui nowrap
    rect w=14 h=1 bg=none
    text "→" size=13 color=color.uiFaint nowrap
    rect w=14 h=1 bg=none
    row w=330 h=20 radius=6 bg=#171719 align=center pad=0,10 {{
      text "agentfs-cxx" size=11.5 color=color.ui nowrap
      spacer
      icon refresh size=11 color=color.uiDim
      rect w=6 h=1 bg=none
      text "⌄" size=10 color=color.uiFaint nowrap
    }}
    rect w=10 h=1 bg=none
    rect w=15 h=15 radius=999 stroke=color.uiDim stroke-w=1.5 {{ }}
    spacer
    icon grid4 size=11 color=color.uiDim
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
acts = ['files','search','branch','play','warn','gear']
A(f'''    col w=55 bg=color.bg2 align=center pad=10,0 gap=0 {{
      col h=34 align=center {{ icon files size=20 color=#C9C9CC }}
      col h=34 align=center {{ icon search size=19 color=color.uiFaint }}
      col h=34 align=center {{ icon branch size=19 color=color.uiFaint }}
      col h=34 align=center {{ icon play size=19 color=color.uiFaint }}
      col h=34 align=center {{ rect w=17 h=13 radius=2 stroke=color.uiFaint stroke-w=1.5 {{ }} }}
      col h=34 align=center {{
        stack w=20 h=20 {{
          rect w=15 h=15 radius=2 stroke=color.uiFaint stroke-w=1.5
          rect w=9 h=9 radius=999 bg={C['yellow']} self=bottom-end
        }}
      }}
      col h=34 align=center {{ rect w=14 h=14 radius=2 stroke=color.uiFaint stroke-w=1.5 {{ }} }}
      col h=34 align=center {{ icon warn size=18 color=color.uiFaint }}
      col h=34 align=center {{ path "M12 3 L21 12 L12 21 L3 12 Z M12 8 L12 16 M8 12 L16 12" w=18 h=18 bg=none stroke=color.uiFaint stroke-w=1.5 }}
      spacer
      col h=34 align=center {{ rect w=16 h=16 radius=999 stroke=color.uiDim stroke-w=1.5 {{ }} }}
    }}''')

# sidebar
A(f'''    col #sidebar w=fill:195 min-w=170 max-w=560 bg=color.bg2 gap=0 clip {{
      row h=30 align=center pad=0,16 {{
        text "EXPLORER" size=10.5 color=color.uiDim tracking=0.4 nowrap
        spacer
        text "···" size=11 color=color.uiDim nowrap
      }}
      row h=20 align=center pad=0,6 gap=3 {{
        icon chev-d size=8 color=color.uiDim
        text "AGENTFS-CXX" size=10.5 weight=700 color=color.ui tracking=0.3 nowrap
      }}''')
A(tree_rows())
A(f'''      spacer
      row h=15 align=center pad=0,6 gap=3 {{ icon chev-r size=8 color=color.uiFaint; text "OUTLINE" size=10.5 weight=700 color=color.uiDim tracking=0.3 nowrap }}
      row h=15 align=center pad=0,6 gap=3 {{ icon chev-r size=8 color=color.uiFaint; text "TIMELINE" size=10.5 weight=700 color=color.uiDim tracking=0.3 nowrap }}
      rect h=12 bg=none
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
        row h=25 align=center pad=0,14 gap=16 {{
          row gap=5 align=center h=fill act=panel_pick key=problems color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "PROBLEMS" size=10.5 nowrap
            row w=15 h=15 radius=999 bg={C['red']} align=center pack=center {{ text "6" size=9 color=#FFFFFF nowrap }}
          }}
          col h=fill pack=center act=panel_pick key=output color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "OUTPUT" size=10.5 nowrap
          }}
          col h=fill pack=center act=panel_pick key=debug color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "DEBUG CONSOLE" size=10.5 nowrap
          }}
          col h=fill pack=center act=panel_pick key=terminal color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "TERMINAL" size=10.5 nowrap
          }}
          col h=fill pack=center act=panel_pick key=ports color=color.uiDim {{
            when selected {{ color=#D0D0D2 stroke=#C9C9CC stroke-w=1.5 stroke-sides=b }}
            text "PORTS" size=10.5 nowrap
          }}
          spacer
          row w=155 h=18 radius=3 bg=#1A191E align=center pad=0,7 {{
            text "Filter (e.g. text, !excludeTex..." field=filter_change size=10 color=color.uiFaint nowrap
          }}
          row gap=4 align=center {{
            text "C/C++ Configuration Warn" size=10.5 color=color.ui nowrap
            text "⌄" size=9 color=color.uiFaint nowrap
          }}
          icon lines size=11 color=color.uiDim
          icon expand size=11 color=color.uiDim
          icon close size=9 color=color.uiDim
        }}
        col w=fill h=fill pad=4,14 gap=0 family="JetBrains Mono" leading=1.32 select select-bg=#264F7866 {{
          para {{ span "[7/30/2026, 5:08:54 PM] For C source files, IntelliSenseMode was changed from \\"macos-clang-x64\\" to \\"macos-clang-arm64\\" based on compiler args and querying compilerPath: \\"/usr/bin/clang\\"" color=#BEBDC0 size=10.5 }}
          para {{ span "[7/30/2026, 5:08:54 PM] IntelliSenseMode was changed because it didn't match the detected compiler.  Consider setting \\"compilerPath\\" instead.  Set \\"compilerPath\\" to \\"\\" to disable detection of system includes and defines." color=#BEBDC0 size=10.5 }}
          para {{ span "[7/30/2026, 5:08:54 PM] For C++ source files, IntelliSenseMode was changed from \\"macos-clang-x64\\" to \\"macos-clang-arm64\\" based on compiler args and querying compilerPath: \\"/usr/bin/clang\\"" color=#BEBDC0 size=10.5 }}
          para {{ span "[7/30/2026, 5:08:54 PM] IntelliSenseMode was changed because it didn't match the detected compiler.  Consider setting \\"compilerPath\\" instead.  Set \\"compilerPath\\" to \\"\\" to disable detection of system includes and defines." color=#BEBDC0 size=10.5 }}
        }}
      }}
    }}''')  # end editors+panel col

# ---- chat panel
A(f'''    divider #sashR w=4 bg=none label="Resize chat" stroke=color.sep stroke-w=0.5 stroke-sides=l resize=sash_center {{
      when hover {{ bg=#3C7BACAA }}
      when pressed {{ bg=#3C7BACDD }}
    }}
    col #chat w=fill:198 min-w=150 max-w=640 bg=color.bg2 gap=0 clip {{
      row h=26 align=center pad=0,10 gap=8 {{
        text "CHAT" size=10.5 weight=600 color=#C6C0C4 tracking=0.3 nowrap
        spacer
        text "+" size=13 color=color.uiDim nowrap
        icon gear size=11 color=color.uiDim
        text "···" size=11 color=color.uiDim nowrap
        icon expand size=10 color=color.uiDim
        icon close size=8 color=color.uiDim
      }}
      row h=22 align=center pad=0,10 {{
        text "SESSIONS" size=10 color=color.uiDim tracking=0.4 nowrap
        spacer
        icon search size=10 color=color.uiFaint
        rect w=8 h=1 bg=none
        icon lines size=10 color=color.uiFaint
      }}
      spacer
      col pad=0,10 gap=3 {{
        para {{
          span "Tip: " color=#B9B9BC size=10.5
          span "Try the " color=color.uiDim size=10.5
          span "Plan agent" color={C['red']} size=10.5
          span " to research and plan before implementing changes." color=color.uiDim size=10.5
        }}
      }}
      rect h=8 bg=none
      col pad=0,8 gap=0 {{
        col radius=8 bg={C['cardBg']} stroke=#232228 stroke-w=1 pad=8 gap=7 {{
          row gap=4 align=center {{
            text "+" size=11 color=color.uiDim nowrap
            text "h" size=9 weight=700 color=color.blue nowrap family="JetBrains Mono"
            text "store.hpp" size=10 color=color.uiDim italic nowrap
          }}
          text "Describe what to build" field=chat_change size=11.5 color=#55555A nowrap
          row gap=7 align=center {{
            text "+" size=12 color=color.uiDim nowrap
            text "∞" size=11 color=color.uiDim nowrap
            text "Agent" size=10.5 color=#B9B9BC nowrap
            icon circ size=9 color=color.uiDim
            text "Auto" size=10.5 color=#B9B9BC nowrap
            icon swap size=10 color=color.uiDim
            spacer
            icon mic size=10 color=color.uiDim
            icon enter size=10 color=color.uiDim
          }}
        }}
      }}
      row h=26 align=center pad=0,12 gap=5 {{
        icon square2 size=9 color=color.uiDim
        text "Local" size=10.5 color=color.uiDim nowrap
        rect w=8 h=1 bg=none
        icon circ size=9 color=color.uiDim
        text "Default approvals" size=10.5 color=color.uiDim nowrap
      }}
    }}''')

A('  }')  # end main row

# ---- status bar
A(f'''  row h=22 align=center bg=color.bg pad=0,10 gap=0 {{
    icon zap size=12 color=color.ui
    rect w=14 h=1 bg=none
    icon errc size=12 color=color.uiDim
    rect w=3 h=1 bg=none
    text "4" size=10.5 color=color.uiDim nowrap
    rect w=6 h=1 bg=none
    icon warn size=12 color=color.uiDim
    rect w=3 h=1 bg=none
    text "0" size=10.5 color=color.uiDim nowrap
    rect w=14 h=1 bg=none
    icon gear size=11 color=color.uiDim
    rect w=4 h=1 bg=none
    text "Build" size=10.5 color=color.uiDim nowrap
    rect w=14 h=1 bg=none
    icon grid4 size=10 color=color.uiDim
    rect w=14 h=1 bg=none
    icon play size=11 color=color.uiDim
    rect w=8 h=1 bg=none
    text "Auto Attach: With Flag" size=10.5 color=color.uiDim nowrap
    spacer
    text "Ln 1, Col 1" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "Tab Size: 3" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "UTF-8" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "LF" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "{{}} C++" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "Mac" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    text "⊘ Prettier" size=10.5 color=color.uiDim nowrap
    rect w=16 h=1 bg=none
    icon bell size=11 color=color.uiDim
  }}
  rect h=3 bg=#000000
}}''')

open(__file__.replace('gen.py', 'vscode.slab'), 'w').write('\n'.join(doc) + '\n')
print('wrote vscode.slab,', len('\n'.join(doc).splitlines()), 'lines')
