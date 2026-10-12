.PHONY: all clean

# Writes past the tag's disk limit while building, so that the suite can check
# that the build is stopped with a disk limit status rather than reported as an
# ordinary build failure.
all:
	dd if=/dev/zero of=fill bs=1M count=1024

clean:
	$(RM) fill
