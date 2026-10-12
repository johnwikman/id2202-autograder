.PHONY: all clean

# Builds a `hello` that writes past the tag's disk limit when a test runs it,
# so that the suite can check that the tag is stopped with a disk limit status
# rather than reported as a failed test.
all:
	printf '#!/bin/sh\ndd if=/dev/zero of=fill bs=1M count=1024\n' > hello
	chmod +x hello

clean:
	$(RM) hello fill
