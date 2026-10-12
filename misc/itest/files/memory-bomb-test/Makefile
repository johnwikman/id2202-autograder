.PHONY: all clean

# Builds a `hello` that allocates past the tag's memory limit when a test runs
# it, so that the suite can check that the tag is stopped with a memory limit
# status rather than reported as a failed test. GNU tail keeps all of /dev/zero
# in memory, looking for a newline that never comes.
all:
	printf '#!/bin/sh\ntail /dev/zero\n' > hello
	chmod +x hello

clean:
	$(RM) hello
