(*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the "hack" directory of this source tree.
 *
 *)

open Sdt_analysis_types

let parse_command = function
  | "dump" -> Some Options.DumpConstraints
  | "solve" -> Some Options.SolveConstraints
  | "dump-persisted" -> Some Options.DumpPersistedConstraints
  | "solve-persisted" -> Some Options.SolvePersistedConstraints
  | _ -> None

let mk ~command ~verbosity = Options.{ command; verbosity }
