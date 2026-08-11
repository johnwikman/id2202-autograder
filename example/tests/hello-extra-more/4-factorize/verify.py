import math
import autograder_verifier_tools as avt

run = avt.read_stdin()

if run.param["expect_fail"]:
    avt.expect(run.code == 1, "invalid return code")
    avt.accept()

avt.expect(run.code == 0, "invalid return code")

assert len(run.cmd) == 3
assert run.cmd[1] == "factorize"
in_value = int(run.cmd[2])

with avt.no_except("bad format of output values"):
    out_values = [int(sv) for sv in run.stdout.as_utf8().split()]

avt.expect(math.prod(out_values) == in_value, "invalid factorization")

# Check that the factors are either prime or -1
def is_prime(x):
    limit = int(math.sqrt(x) * 1.1 + 1) if x > 10 else x
    if x < 2:        return False
    elif x == 2:     return True
    elif x % 2 == 0: return False
    for d in range(3, limit, 2):
        if x % d == 0:
            return False
    return True

if not all(is_prime(ov) or ov == -1 for ov in out_values):
    avt.reject("invalid factors, only prime factors and -1 allowed")

avt.accept()
