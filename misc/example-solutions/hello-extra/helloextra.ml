let hello_stdout = ref false
let hello_stderr = ref false
let hello_stdin2stdout = ref false
let hello_code = ref 0

let input_files = ref []
let input_file_accum path =
  input_files := path :: !input_files

let speclist =
  [("--stdout", Arg.Set hello_stdout, "Print hello world to stdout.");
   ("--stderr", Arg.Set hello_stderr, "Print hello world to stderr.");
   ("--stdin-to-stdout", Arg.Set hello_stdin2stdout, "Reroutes stdin to stdout.");
   ("--code", Arg.Set_int hello_code, "Set return code.")]

let main =
  let _ =
    Arg.parse speclist input_file_accum "usage: see source code."
  in
  let _ =
    if !hello_stdout then
      print_endline "Hello, World!"
    else ()
  in
  let _ =
    if !hello_stderr then
       prerr_endline "Hello, World!"
    else ()
  in
  let _ =
    if !hello_stdin2stdout then
      let s = In_channel.input_all stdin in
      print_string s
    else ()
  in
  let _ =
    List.iter (fun path ->
      let f = In_channel.open_text path in
      let s = In_channel.input_all f in
      In_channel.close f;
      print_string s;
      ()
    ) (!input_files)
  in
  exit !hello_code
