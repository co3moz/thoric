"""Thoric's own license, GPL-3.0-or-later: that every place that states it states the same
license, and that every image and package a release distributes carries its notice.
"""

import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPDX = "GPL-3.0-or-later"
OR_LATER = "either version 3 of the License, or (at your option) any later version"


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def stages(dockerfile):
    """Each stage of a Dockerfile by its name, with its instructions."""
    found = {}
    for block in re.split(r"^FROM ", read(dockerfile), flags=re.M)[1:]:
        name = re.match(r"\S+(?:\s+AS\s+(\S+))?", block, re.I).group(1)
        found[name or ""] = block
    return found


def workflow_steps(workflow):
    """Each step of a workflow that has an `id`, by that id, with its text."""
    found = {}
    for block in re.split(r"(?m)^ *- (?=name:|uses:)", read(workflow))[1:]:
        step = re.search(r"(?m)^ *id: (\S+)$", block)
        if step:
            found[step.group(1)] = block
    return found


def labels(step):
    """A step's `labels` input: the lines of its block, or the one value it passes on."""
    block = re.search(r"(?m)^( *)labels: \|\n((?:\1 +\S.*\n)+)", step)
    if block:
        return [line.strip() for line in block.group(2).splitlines()]
    value = re.search(r"(?m)^ *labels: (.+)$", step)
    return [value.group(1).strip()] if value else []


class LicenseTest(unittest.TestCase):
    def test_the_license_file_is_the_gplv3(self):
        text = read("LICENSE")
        self.assertIn("GNU GENERAL PUBLIC LICENSE", text.splitlines()[0])
        self.assertIn("Version 3, 29 June 2007", text.splitlines()[1])
        self.assertIn("END OF TERMS AND CONDITIONS", text)

    def test_every_license_field_names_the_same_license(self):
        self.assertRegex(read("core/manager/Cargo.toml"), rf'(?m)^license = "{re.escape(SPDX)}"$')
        self.assertEqual(json.loads(read("core/web/package.json"))["license"], SPDX)
        self.assertIn(f'org.opencontainers.image.licenses="{SPDX}"', read("core/manager/Dockerfile"))

    def test_every_published_image_is_labeled_with_the_license(self):
        # docker/metadata-action adds the license GitHub detects, `GPL-3.0`, and the labels it
        # hands the build override the Dockerfile's, so each image's own step names the license.
        steps = workflow_steps(".github/workflows/publish.yml")
        builds = {name: step for name, step in steps.items() if "uses: docker/build-push-action@" in step}
        self.assertTrue(builds)
        for build, step in builds.items():
            meta = re.fullmatch(r"\$\{\{ steps\.(\S+)\.outputs\.labels \}\}", " ".join(labels(step)))
            self.assertIsNotNone(meta, build)
            self.assertIn("uses: docker/metadata-action@", steps[meta.group(1)], build)
            self.assertIn(f"org.opencontainers.image.licenses={SPDX}", labels(steps[meta.group(1)]), build)

    def test_every_notice_names_version_3_or_later(self):
        for path in ("README.md", "images/desktop/copyright"):
            self.assertIn(OR_LATER, " ".join(read(path).split()), path)
        self.assertIn("version 3 or later", read("THIRD_PARTY_NOTICES.md"))
        # The helpers are attached to releases on their own, away from the repository's LICENSE.
        for helper in ("packaging/windows/thoric.ps1", "packaging/unix/thoric.sh"):
            self.assertIn("GNU General Public\n# License, version 3 or later", read(helper).replace("\r\n", "\n"), helper)

    def test_every_image_and_package_carries_the_notice(self):
        self.assertIn("COPY LICENSE /usr/share/doc/thoric/LICENSE", read("core/manager/Dockerfile"))
        self.assertIn("COPY --chmod=0644 LICENSE /usr/share/doc/thoric/LICENSE", read("packaging/wsl/Dockerfile"))
        desktop = stages("images/desktop/Dockerfile")
        self.assertIn("COPY copyright /usr/share/doc/thoric/copyright", desktop["scripts"])
        for image in ("xfce", "gnome"):
            self.assertIn("COPY --from=scripts /usr/share/doc/thoric/ /usr/share/doc/thoric/", desktop[image], image)


if __name__ == "__main__":
    unittest.main()
