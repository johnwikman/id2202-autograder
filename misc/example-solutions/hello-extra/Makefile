.PHONY: all clean
all:
	ocamlbuild -package unix helloextra.native
	mv helloextra.native hello-extra

clean:
	$(RM) -rf _build hello-extra
