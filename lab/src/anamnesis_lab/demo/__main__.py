import sys

from ..cli import main

sys.exit(main(["demo", *sys.argv[1:]]))
