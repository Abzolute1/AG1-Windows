//! Compiler selection preserves the pinned check-detect precedence. No compiler
//! is downloaded by check. Cargo JSON is a deliberate richer diagnostic format.
use super::{Command, Toolchains};
use crate::workspace::Workspace;
use ag1_core::DomainError;
use serde_json::Value;
use std::path::PathBuf;
pub fn detect(workspace: &Workspace, tools: &Toolchains) -> Result<(String, Command), DomainError> {
    let exists = |p: &str| workspace.exists(p);
    let missing = |name: &str| {
        DomainError::Unavailable(format!(
            "Configure installed {name}; check never installs a compiler"
        ))
    };
    let system =
        |name: &str, args: &[&str], kind: &str| -> Result<(String, Command), DomainError> {
            let executable = PathBuf::from("/usr/bin").join(name);
            if !executable.is_file() {
                return Err(missing(name));
            }
            Ok((
                kind.into(),
                Command {
                    executable,
                    argv: args.iter().map(|s| s.to_string()).collect(),
                    cwd: String::new(),
                },
            ))
        };
    let package: Option<Value> = if exists("package.json")? {
        Some(
            serde_json::from_slice(&workspace.snapshot("package.json", 1024 * 1024)?.bytes)
                .map_err(|_| DomainError::InvalidInput("package.json".into()))?,
        )
    } else {
        None
    };
    if let Some(p) = &package {
        let script = if p["scripts"]["typecheck"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        {
            Some("typecheck")
        } else if p["scripts"]["tsc"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        {
            Some("tsc")
        } else if p["workspaces"].as_array().is_some_and(|a| !a.is_empty())
            && p["scripts"]["build"].is_string()
        {
            Some("build")
        } else {
            None
        };
        if let Some(script) = script {
            return Ok((
                "typescript".into(),
                Command {
                    executable: tools
                        .node
                        .clone()
                        .filter(|p| p.is_file())
                        .ok_or_else(|| missing("Node"))?,
                    argv: vec![
                        tools
                            .npm
                            .as_ref()
                            .filter(|p| p.is_file())
                            .ok_or_else(|| missing("npm"))?
                            .display()
                            .to_string(),
                        "run".into(),
                        script.into(),
                        "--silent".into(),
                    ],
                    cwd: String::new(),
                },
            ));
        }
    }
    if exists("tsconfig.json")?
        || package.as_ref().is_some_and(|p| {
            !p["dependencies"]["typescript"].is_null()
                || !p["devDependencies"]["typescript"].is_null()
        })
    {
        let local = workspace.root().join("node_modules/typescript/bin/tsc");
        if local.is_file() {
            return Ok((
                "typescript".into(),
                Command {
                    executable: tools
                        .node
                        .clone()
                        .filter(|p| p.is_file())
                        .ok_or_else(|| missing("Node"))?,
                    argv: vec![
                        local.display().to_string(),
                        "--noEmit".into(),
                        "--pretty".into(),
                        "false".into(),
                    ],
                    cwd: String::new(),
                },
            ));
        }
        return system("tsc", &["--noEmit", "--pretty", "false"], "typescript");
    }
    if exists("Cargo.toml")? {
        return Ok((
            "rust".into(),
            Command {
                executable: tools
                    .cargo
                    .clone()
                    .filter(|p| p.is_file())
                    .ok_or_else(|| missing("Cargo (rustup)"))?,
                argv: vec!["check".into(), "--message-format=json".into()],
                cwd: String::new(),
            },
        ));
    }
    if exists("go.mod")? {
        return system("go", &["build", "-buildvcs=false", "./..."], "go");
    }
    if exists("mypy.ini")?
        || exists(".mypy.ini")?
        || (exists("pyproject.toml")?
            && String::from_utf8_lossy(&workspace.snapshot("pyproject.toml", 1024 * 1024)?.bytes)
                .contains("mypy"))
    {
        return system("mypy", &["."], "python");
    }
    if workspace
        .directory_entries("")?
        .iter()
        .any(|entry| entry.name.ends_with(".csproj"))
    {
        return system("dotnet", &["build", "--nologo", "-v", "q"], "csharp");
    }
    for (manifest, compiler, markers, dirs) in [
        (
            "CMakeLists.txt",
            "cmake",
            vec!["CMakeCache.txt", "build.ninja", "Makefile"],
            vec![
                "build",
                "cmake-build-debug",
                "cmake-build-release",
                "out",
                "_build",
                "builddir",
            ],
        ),
        (
            "meson.build",
            "meson",
            vec!["meson-private", "build.ninja"],
            vec!["build", "_build", "builddir"],
        ),
    ] {
        if !exists(manifest)? {
            continue;
        }
        for dir in dirs {
            for marker in &markers {
                if exists(&format!("{dir}/{marker}"))? {
                    return if compiler == "cmake" {
                        system(compiler, &["--build", dir], "cxx")
                    } else {
                        system(compiler, &["compile", "-C", dir], "cxx")
                    };
                }
            }
        }
        return Err(DomainError::Unavailable(
            if compiler == "cmake" {
                "Configure a CMake build directory (cmake -S . -B build)"
            } else {
                "Configure a Meson build directory (meson setup build)"
            }
            .into(),
        ));
    }
    if exists("build.ninja")? {
        return system("ninja", &[], "cxx");
    }
    let cxx = regex::Regex::new(r"\$\(CC\)|\$\(CXX\)|\bgcc\b|\bg\+\+\b|\bclang|\.(c|cc|cpp|cxx)\b")
        .unwrap();
    let pattern = regex::Regex::new(r"(?m)^([A-Za-z0-9_./-]+)\s*:").unwrap();
    for makefile in ["Makefile", "makefile", "GNUmakefile"] {
        if exists(makefile)? {
            let bytes = workspace.snapshot(makefile, 1024 * 1024)?.bytes;
            let text = String::from_utf8_lossy(&bytes);
            if !cxx.is_match(&text) {
                continue;
            }
            let targets = pattern
                .captures_iter(&text)
                .map(|c| c[1].to_string())
                .filter(|t| {
                    ![
                        ".PHONY",
                        "clean",
                        "distclean",
                        "install",
                        "uninstall",
                        "test",
                        "check",
                        "help",
                        "fmt",
                        "format",
                        "lint",
                    ]
                    .contains(&t.as_str())
                })
                .collect::<Vec<_>>();
            if let Some(target) = targets
                .iter()
                .find(|t| *t == "all")
                .or_else(|| targets.iter().find(|t| *t == "build"))
                .or_else(|| targets.first())
            {
                return system("make", &[target], "cxx");
            }
        }
    }
    if exists("compile_commands.json")? {
        return Err(DomainError::Unavailable(
            "Configure a CMake/Ninja/Make build to consume compile_commands.json".into(),
        ));
    }
    Err(DomainError::Unavailable(
        "No project compiler/typechecker detected; configure a target command".into(),
    ))
}
