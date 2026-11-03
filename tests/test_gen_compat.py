import struct
import unittest

from tools import gen_compat


def u2(value):
    return struct.pack(">H", value)


def u4(value):
    return struct.pack(">I", value)


def utf8(text):
    encoded = text.encode("utf-8")
    return b"\x01" + u2(len(encoded)) + encoded


def class_entry(name_index):
    return b"\x07" + u2(name_index)


def synthetic_class():
    # Our own minimal Java class with one field and one method. The method has
    # a Code attribute so this specifically checks that the signatures-only
    # reader skips, rather than inspects, bytecode.
    pool = [
        utf8("dev/morizen/test/Example"),
        class_entry(1),
        utf8("java/lang/Object"),
        class_entry(3),
        utf8("value"),
        utf8("I"),
        utf8("run"),
        utf8("()V"),
        utf8("Code"),
    ]
    field = u2(0x0001) + u2(5) + u2(6) + u2(0)
    code = u2(0) + u2(1) + u4(1) + b"\xb1" + u2(0) + u2(0)
    method = u2(0x0001) + u2(7) + u2(8) + u2(1) + u2(9) + u4(len(code)) + code
    return (
        b"\xca\xfe\xba\xbe"
        + u2(0)
        + u2(69)
        + u2(len(pool) + 1)
        + b"".join(pool)
        + u2(0x0021)
        + u2(2)
        + u2(4)
        + u2(0)
        + u2(1)
        + field
        + u2(1)
        + method
        + u2(0)
    )


class CompatGeneratorTests(unittest.TestCase):
    def test_class_reader_skips_code_and_keeps_signatures(self):
        signature = gen_compat.class_signatures(synthetic_class())
        self.assertEqual(signature["name"], "dev/morizen/test/Example")
        self.assertEqual(signature["super"], "java/lang/Object")
        self.assertEqual(signature["fields"], {("value", "I"): 0x0001})
        self.assertEqual(signature["methods"], {("run", "()V"): 0x0001})

    def test_descriptors_become_java_types(self):
        params, result = gen_compat.split_descriptor(
            "(ILjava/lang/String;[Lnet/minecraft/resources/Identifier;)Z"
        )
        self.assertEqual(
            [gen_compat.type_name(value) for value in params],
            ["int", "java.lang.String", "net.minecraft.resources.Identifier[]"],
        )
        self.assertEqual(gen_compat.type_name(result), "boolean")

    def test_tracked_surface_is_unique_and_pinned(self):
        manifest = gen_compat.surface()
        self.assertEqual(manifest["minecraft"], "26.3")
        names = [entry["name"] for entry in manifest["classes"]]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(len(names), 12)
        for entry in manifest["classes"]:
            members = [
                (member["kind"], member.get("name", "<init>"), member["descriptor"])
                for member in entry["members"]
            ]
            self.assertEqual(len(members), len(set(members)), entry["name"])


if __name__ == "__main__":
    unittest.main()
