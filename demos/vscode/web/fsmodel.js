const dir = (name, path, children, tint = '#909094') => ({
  name,
  path,
  dir: true,
  children,
  letter: '',
  icon: '',
  tint,
  badge: '',
});

const file = (name, path, letter = '', icon = 'blank', tint = '#707074', badge = '') => ({
  name,
  path,
  dir: false,
  letter,
  icon,
  tint,
  badge,
});

export const FILES = [
  dir('build', 'build', []),
  dir('include', 'include', [
    dir('agentfs', 'include/agentfs', [
      file('cid.hpp', 'include/agentfs/cid.hpp', 'h', ''),
      file('launch.hpp', 'include/agentfs/launch.hpp', 'h', ''),
      file('mdb.hpp', 'include/agentfs/mdb.hpp', 'h', '', '#A63D5B', '2'),
      file('node.hpp', 'include/agentfs/node.hpp', 'h', ''),
      file('overlay.hpp', 'include/agentfs/overlay.hpp', 'h', ''),
      file('path.hpp', 'include/agentfs/path.hpp', 'h', ''),
      file('store.hpp', 'include/agentfs/store.hpp', 'h', '', '#A63D5B', '2'),
    ], '#A63D5B'),
  ]),
  dir('scripts', 'scripts', []),
  dir('src', 'src', [
    dir('cli', 'src/cli', [
      file('main.cpp', 'src/cli/main.cpp', 'C', ''),
    ]),
    dir('core', 'src/core', [
      file('launch.cpp', 'src/core/launch.cpp', 'C', ''),
      file('mdb.cpp', 'src/core/mdb.cpp', 'C', ''),
      file('overlay.cpp', 'src/core/overlay.cpp', 'C', ''),
      file('path.cpp', 'src/core/path.cpp', 'C', ''),
      file('store.cpp', 'src/core/store.cpp', 'C', ''),
    ]),
    dir('hook', 'src/hook', [
      file('context.cpp', 'src/hook/context.cpp', 'C', ''),
      file('hook.hpp', 'src/hook/hook.hpp', 'h', ''),
      file('hooks_path.cpp', 'src/hook/hooks_path.cpp', 'C', ''),
      file('hooks_proc.cpp', 'src/hook/hooks_proc.cpp', 'C', ''),
      file('interpose.hpp', 'src/hook/interpose.hpp', 'h', ''),
      file('spawn.cpp', 'src/hook/spawn.cpp', 'C', ''),
      file('spawn.hpp', 'src/hook/spawn.hpp', 'h', ''),
    ]),
    dir('interpose', 'src/interpose', []),
  ]),
  dir('tests', 'tests', []),
  dir('third_party', 'third_party', []),
  file('.clang-format', '.clang-format', '', 'gearfile'),
  file('.gitignore', '.gitignore', '', 'branch'),
  file('CMakeLists.txt', 'CMakeLists.txt', '', 'cmakefile'),
  file('demo.sh', 'demo.sh', '', 'shellfile'),
  file('README.md', 'README.md', '', 'markdown'),
];

export const CONTENTS = {
  'include/agentfs/cid.hpp': `#pragma once
#include <array>
#include <cstddef>
#include <span>
#include <string>

namespace agentfs {
struct cid {
   std::array<std::byte, 32> bytes{};
   friend bool operator==( const cid&, const cid& ) = default;
};
cid hash( std::span<const std::byte> data );
std::string to_string( const cid& value );
} // namespace agentfs`,

  'include/agentfs/launch.hpp': `#pragma once
#include <filesystem>
#include <string>
#include <vector>

namespace agentfs {
struct launch_options {
   std::filesystem::path image;
   std::vector<std::string> command;
   bool writable = false;
};
int launch( const launch_options& options );
} // namespace agentfs`,

  'include/agentfs/mdb.hpp': `#pragma once
#include <lmdb.h>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>
#include <utility>

namespace agentfs::mdb
{
   struct error
   {
      int code = 0;
      std::string_view message() const;
   };

   struct value
   {
      MDB_val raw{};
      std::span<const std::byte> bytes() const;
   };

   struct database
   {
      MDB_dbi handle = 0;
      constexpr explicit operator bool() const { return handle != 0; }
   };

   // Owns an LMDB environment handle.
   //
   struct environment
   {
      MDB_env* handle = nullptr;

   // The handle remains null until open succeeds.
   //
   static environment open( std::string_view path );

   constexpr environment() = default;
   constexpr environment( environment&& other ) noexcept : handle( std::exchange( other
   constexpr environment& operator=( environment&& other ) noexcept
   {
      std::swap( handle, other.handle );
      return *this;
   }
   environment( const environment& )            = delete;
   environment& operator=( const environment& ) = delete;
   ~environment() { reset(); }

   void reset()
   {
      if ( MDB_env* previous = std::exchange( handle, nullptr ) )
         mdb_env_close( previous );
   }
   constexpr explicit operator bool() const { return handle != nullptr; }
};

// Owns a transaction handle and aborts it unless it is committed first.
//
struct transaction
{
   MDB_txn* handle = nullptr;

   constexpr transaction() = default;
   constexpr explicit transaction( MDB_txn* handle ) : handle( handle ) {}
   constexpr transaction( transaction&& other ) noexcept : handle( std::exchange( other
   constexpr transaction& operator=( transaction&& other ) noexcept
   {
      std::swap( handle, other.handle );
      return *this;
   }
   transaction( const transaction& )            = delete;
   transaction& operator=( const transaction& ) = delete;
   ~transaction() { abort(); }

   void abort()
   {
      if ( MDB_txn* previous = std::exchange( handle, nullptr ) )
         mdb_txn_abort( previous );`,

  'include/agentfs/node.hpp': `#pragma once
#include <cstdint>
#include <string>
#include <vector>

namespace agentfs {
enum class node_kind { file, directory, symlink };
struct node_record {
   node_kind kind = node_kind::file;
   uint64_t size = 0;
   uint64_t modified_ns = 0;
   std::vector<std::byte> payload;
};
} // namespace agentfs`,

  'include/agentfs/overlay.hpp': `#pragma once
#include <filesystem>
#include <string_view>
#include "store.hpp"

namespace agentfs {
class overlay {
 public:
   explicit overlay( store& image );
   bool contains( std::string_view path ) const;
   void materialize( const std::filesystem::path& target ) const;
 private:
   store* image_;
};
} // namespace agentfs`,

  'include/agentfs/path.hpp': `#pragma once
#include <string>
#include <string_view>
#include <vector>

namespace agentfs::path {
std::string normalize( std::string_view input );
std::string parent( std::string_view input );
std::string filename( std::string_view input );
std::vector<std::string_view> components( std::string_view input );
bool is_absolute( std::string_view input );
} // namespace agentfs::path`,

  'include/agentfs/store.hpp': `#pragma once
#include <memory>
#include <span>
#include <string>
#include <string_view>
#include <vector>
#include <xstd/result.hpp>
#include "cid.hpp"
#include "mdb.hpp"
#include "node.hpp"

namespace agentfs
{
   // Wall clock nanoseconds, the unit stored in every node record.
   //
   uint64_t now_ns();

   // One entry of a directory listing.
   //
   struct dir_entry
   {
      std::string name;
      node_kind kind = node_kind::file;
   };

   struct reader;
   struct writer;

   // LMDB backed content addressed filesystem image.
   //
   // Three sub-databases make up the image:
   //   \`meta\`  - format marker and chunk geometry.
   //   \`nodes\` - normalized path -> \`node_record\` plus payload.
   //   \`blobs\` - \`cid\` -> chunk bytes, written once per distinct chunk.
   //
   // Reads borrow directly from the memory map, so a \`reader\` must outlive
   // every span it hands out.
   //
   struct store
   {
      mdb::environment env;
      MDB_dbi meta   = 0;`,

  'src/cli/main.cpp': `#include <agentfs/launch.hpp>
#include <iostream>
#include <string>

int main( int argc, char** argv )
{
   if ( argc < 3 ) {
      std::cerr << "usage: agentfs IMAGE COMMAND...\\n";
      return 2;
   }
   agentfs::launch_options options;
   options.image = argv[1];
   options.command.assign( argv + 2, argv + argc );
   return agentfs::launch( options );
}`,

  'src/core/launch.cpp': `#include <agentfs/launch.hpp>
#include <agentfs/overlay.hpp>
#include <agentfs/store.hpp>
#include <stdexcept>

namespace agentfs {
int launch( const launch_options& options )
{
   store image{ options.image };
   overlay mounted{ image };
   return options.command.empty() ? 1 : 0;
}
} // namespace agentfs`,

  'src/core/mdb.cpp': `#include <agentfs/mdb.hpp>
#include <system_error>

namespace agentfs::mdb {
std::string_view error::message() const
{
   return mdb_strerror( code );
}
std::span<const std::byte> value::bytes() const
{
   return { static_cast<const std::byte*>( raw.mv_data ), raw.mv_size };
}
} // namespace agentfs::mdb`,

  'src/core/overlay.cpp': `#include <agentfs/overlay.hpp>
#include <agentfs/path.hpp>

namespace agentfs {
overlay::overlay( store& image ) : image_( &image ) {}
bool overlay::contains( std::string_view path ) const
{
   auto reader = image_->read();
   return reader.contains( path::normalize( path ) );
}
void overlay::materialize( const std::filesystem::path& target ) const
{
   image_->checkout( target );
}
} // namespace agentfs`,

  'src/core/path.cpp': `#include <agentfs/path.hpp>
#include <filesystem>

namespace agentfs::path {
std::string normalize( std::string_view input )
{
   return std::filesystem::path{ input }.lexically_normal().generic_string();
}
std::string filename( std::string_view input )
{
   return std::filesystem::path{ input }.filename().string();
}
bool is_absolute( std::string_view input )
{
   return std::filesystem::path{ input }.is_absolute();
}
} // namespace agentfs::path`,

  'src/core/store.cpp': `#include <agentfs/store.hpp>
#include <chrono>

namespace agentfs {
uint64_t now_ns()
{
   const auto now = std::chrono::system_clock::now().time_since_epoch();
   return std::chrono::duration_cast<std::chrono::nanoseconds>( now ).count();
}
store::store( const std::filesystem::path& image )
{
   open_environment( env, image );
}
} // namespace agentfs`,

  'src/hook/context.cpp': `#include "hook.hpp"
#include <cstdlib>
#include <mutex>

namespace agentfs::hook {
context& current_context()
{
   static context value;
   return value;
}
void initialize()
{
   auto& ctx = current_context();
   ctx.root = std::getenv( "AGENTFS_ROOT" );
}
} // namespace agentfs::hook`,

  'src/hook/hook.hpp': `#pragma once
#include <filesystem>
#include <optional>
#include <string>

namespace agentfs::hook {
struct context {
   std::filesystem::path root;
   bool active = false;
};
context& current_context();
void initialize();
std::optional<std::filesystem::path> redirect( const char* path );
} // namespace agentfs::hook`,

  'src/hook/hooks_path.cpp': `#include "hook.hpp"
#include "interpose.hpp"
#include <fcntl.h>

extern "C" int open( const char* path, int flags, ... )
{
   auto target = agentfs::hook::redirect( path );
   const char* resolved = target ? target->c_str() : path;
   return agentfs::hook::real_open()( resolved, flags );
}

extern "C" int access( const char* path, int mode )
{
   auto target = agentfs::hook::redirect( path );
   return agentfs::hook::real_access()( target ? target->c_str() : path, mode );
}`,

  'src/hook/hooks_proc.cpp': `#include "hook.hpp"
#include "spawn.hpp"
#include <unistd.h>

extern "C" int execve( const char* file, char* const argv[], char* const envp[] )
{
   return agentfs::hook::spawn( file, argv, envp );
}

extern "C" pid_t fork()
{
   return agentfs::hook::real_fork()();
}`,

  'src/hook/interpose.hpp': `#pragma once
#include <dlfcn.h>
#include <mutex>

namespace agentfs::hook {
template<class Function>
Function resolve( const char* name )
{
   return reinterpret_cast<Function>( dlsym( RTLD_NEXT, name ) );
}
using open_fn = int (*)( const char*, int, ... );
using access_fn = int (*)( const char*, int );
using fork_fn = pid_t (*)();
open_fn real_open();
access_fn real_access();
fork_fn real_fork();
} // namespace agentfs::hook`,

  'src/hook/spawn.cpp': `#include "spawn.hpp"
#include "interpose.hpp"
#include <vector>

namespace agentfs::hook {
int spawn( const char* file, char* const argv[], char* const envp[] )
{
   auto environment = augment_environment( envp );
   using execve_fn = int (*)( const char*, char* const[], char* const[] );
   auto next = resolve<execve_fn>( "execve" );
   return next( file, argv, environment.data() );
}
} // namespace agentfs::hook`,

  'src/hook/spawn.hpp': `#pragma once
#include <string>
#include <vector>

namespace agentfs::hook {
using environment = std::vector<char*>;
environment augment_environment( char* const envp[] );
int spawn(
   const char* file,
   char* const argv[],
   char* const envp[] );
} // namespace agentfs::hook`,

  '.clang-format': `BasedOnStyle: LLVM
IndentWidth: 3
TabWidth: 3
UseTab: Never
ColumnLimit: 100
BreakBeforeBraces: Allman
AllowShortFunctionsOnASingleLine: Empty
PointerAlignment: Left
ReferenceAlignment: Left
SortIncludes: CaseSensitive
SpaceBeforeParens: ControlStatements`,

  '.gitignore': `build/
.cache/
compile_commands.json
.DS_Store
*.swp
*.swo
*.dylib
*.so
*.o
agentfs.img
coverage/`,

  'CMakeLists.txt': `cmake_minimum_required(VERSION 3.24)
project(agentfs-cxx LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(agentfs
  src/core/launch.cpp
  src/core/mdb.cpp
  src/core/overlay.cpp
  src/core/path.cpp
  src/core/store.cpp)
target_include_directories(agentfs PUBLIC include)
target_link_libraries(agentfs PUBLIC lmdb)

add_executable(agentfs-cli src/cli/main.cpp)
target_link_libraries(agentfs-cli PRIVATE agentfs)`,

  'demo.sh': `#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
image="\${TMPDIR:-/tmp}/agentfs-demo.mdb"
rm -rf "$image"
cmake -S "$root" -B "$root/build"
cmake --build "$root/build"
exec "$root/build/agentfs-cli" "$image" /bin/sh`,

  'README.md': `# agentfs-cxx

A content-addressed filesystem image for isolated build agents.

## Build

\`\`\`sh
cmake -S . -B build
cmake --build build
\`\`\`

Run \`./demo.sh\` to create an image and launch a shell backed by it.`,
};

export const DEFAULT_OPEN = new Set([
  'include',
  'include/agentfs',
  'src',
  'src/cli',
  'src/core',
  'src/hook',
]);

export function visibleRows(openSet) {
  const rows = [];

  const visit = (nodes, depth) => {
    for (const node of nodes) {
      rows.push({
        key: node.path,
        name: node.name,
        letter: node.letter,
        icon: node.dir ? (openSet.has(node.path) ? 'folder-open' : 'folder') : node.icon,
        tint: node.tint,
        badge: node.badge,
        indent: 14 + depth * 10,
        dir: node.dir,
        open: openSet.has(node.path),
      });
      if (node.dir && openSet.has(node.path)) visit(node.children, depth + 1);
    }
  };

  visit(FILES, 0);
  return rows;
}

const KEYWORDS = new Set([
  'constexpr', 'explicit', 'operator', 'const', 'return', 'if', 'void', 'struct',
  'delete', 'default', 'noexcept', 'namespace', 'using', 'bool', 'nullptr',
  'this', 'class', 'public',
]);

const TYPES = new Set([
  'uint64_t', 'MDB_env', 'MDB_txn', 'MDB_dbi', 'node_kind', 'dir_entry', 'size_t',
]);

const TOKENS = /\/\/.*$|"[^"]*"|<[^>\s]+>|[A-Za-z_][A-Za-z0-9_]*|\d+|./gu;

export function highlight(text) {
  const ranges = [];
  const lines = text.split('\n');
  let lineStart = 0;

  const add = (start, end, color) => {
    const previous = ranges[ranges.length - 1];
    if (previous && previous.end === start && previous.color === color) {
      previous.end = end;
    } else {
      ranges.push({ start, end, color });
    }
  };

  const tokenize = (line) => Array.from(line.matchAll(TOKENS), (match) => match[0]);
  const length = (value) => Array.from(value).length;

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const line = lines[lineIndex];
    const preprocessor = line.match(/^(\s*)(#\w+)(.*)$/u);

    if (line.trimStart().startsWith('#') && preprocessor) {
      const directive = preprocessor[1] + preprocessor[2];
      const directiveEnd = lineStart + length(directive);
      add(lineStart, directiveEnd, '#9D74BE');

      let offset = directiveEnd;
      for (const token of tokenize(preprocessor[3])) {
        const end = offset + length(token);
        if (token.startsWith('"') || token.startsWith('<')) {
          add(offset, end, '#DCA9A8');
        }
        offset = end;
      }
    } else {
      const tokens = tokenize(line);
      let offset = lineStart;

      for (let index = 0; index < tokens.length; index += 1) {
        const token = tokens[index];
        const end = offset + length(token);

        if (token.startsWith('//')) {
          add(offset, end, '#46454A');
        } else if (token.startsWith('"')) {
          add(offset, end, '#DCA9A8');
        } else if (KEYWORDS.has(token)) {
          add(offset, end, '#9D74BE');
        } else if (TYPES.has(token) || /^\d+$/u.test(token)) {
          add(offset, end, '#DCB99E');
        } else if (/^[A-Za-z_]/u.test(token)) {
          let next = index + 1;
          while (next < tokens.length && tokens[next] === ' ') next += 1;
          if (next < tokens.length && tokens[next] === '(') {
            add(offset, end, '#5E9AA0');
          }
        }

        offset = end;
      }
    }

    lineStart += length(line);
    if (lineIndex + 1 < lines.length) lineStart += 1;
  }

  return ranges;
}
