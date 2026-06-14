(* To parse command-line arguments, checkout OCamls `Arg` module
   (https://ocaml.org/manual/5.1/api/Arg.html). To control the exit code on
   errors you can use `Arg.parse_argv` instead of `Arg.parse` *)

let hello_code = ref 0

let input_files = ref []
let input_file_accum path =
  input_files := path :: !input_files

let speclist =
  [("--code", Arg.Set_int hello_code, "Set return code.")]

let main =
  let _ =
    Arg.parse speclist input_file_accum "usage: see source code."
  in
  (* Print the input files to stderr *)
  let _ =
    List.iter (fun path ->
      let f = In_channel.open_text path in
      let s = In_channel.input_all f in
      In_channel.close f;
      prerr_string s;
      ()
    ) (!input_files)
  in
  (* Print a fixed assembly program *)
  let _ = print_endline "
        global  main
        extern  puts
        extern  fflush

        section .text

hello_str       db      \"Hello, World!\", 0

main:
        mov     rdi, hello_str
        call    puts
        mov     rdi, 0
        call    fflush
        ret
"
  in
  exit !hello_code
