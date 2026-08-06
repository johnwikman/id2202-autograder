let print_exit ?(fn=prerr_endline) code msg = fn msg; exit code

(* ./hello-extra echo ... *)
let mode_echo () =
  let input_files = ref [] in
  let input_file_accum path = input_files := path :: !input_files in
  let write_stderr = ref false in
  let speclist = [("--stderr", Arg.Set write_stderr, "Output to stderr instead.")] in
  let _ = try
    Arg.parse_argv ~current:(ref 1) Sys.argv speclist input_file_accum "usage: see task"
  with
    | Arg.Help msg -> print_exit ~fn:print_string 0 msg
    | Arg.Bad msg -> print_exit ~fn:prerr_string 1 msg
  in
  let print = if !write_stderr then prerr_string else print_string in

  let file_contents = match !input_files with
    | [path] -> (
        let s = try (
          let f = In_channel.open_text path in
          let s = In_channel.input_all f in
          In_channel.close f;
          if not (String.is_valid_utf_8 s) then
            print_exit 1 "file is not valid UTF-8"
          else s
        ) with _ ->
            print_exit 1 "could not read contents from provided file"
        in
        Some s
      )
    | [] -> None
    | _ -> print_exit 1 "more than one input file provided"
  in
  (if not (Unix.isatty Unix.stdin) then
    print (In_channel.input_all stdin)
  else ());
  Option.iter print file_contents

(* ./hello-extra calc OP LHS RHS *)
let mode_calc () =
  if Array.length Sys.argv <> 5 then
    print_exit 1 "usage: ./hello-extra calc OP LHS RHS"
  else
    let (lhs, rhs) = match List.map int_of_string_opt [Sys.argv.(3); Sys.argv.(4)] with
      | [Some lhs; Some rhs] -> (lhs, rhs)
      | _ -> print_exit 1 "lhs and rhs are not integers"
    in
    let res =
      match Sys.argv.(2) with
      | "add" -> lhs + rhs
      | "sub" -> lhs - rhs
      | "mul" -> lhs * rhs
      | _ -> print_exit 1 "invalid calc operator"
    in
    print_endline (string_of_int res)

(* ./hello-extra code VALUE *)
let mode_code () =
  if Array.length Sys.argv <> 3 then
    print_exit 1 "usage: ./hello-extra code VALUE"
  else match int_of_string_opt Sys.argv.(2) with
    | Some code -> exit code
    | None -> print_exit 1 "provided exit code not an integer"


let () =
  if Array.length Sys.argv < 2 then
    print_endline "Hello, World!"
  else match Sys.argv.(1) with
    | "echo" -> mode_echo ()
    | "calc" -> mode_calc ()
    | "code" -> mode_code ()
    | _ -> print_exit 1 "Invalid mode"
