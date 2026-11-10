import importlib.util
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("mission_dll_inventory.py")
SPEC = importlib.util.spec_from_file_location("mission_dll_inventory", SCRIPT)
inventory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(inventory)


class MissionDllInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temp.name)
        self.api = self.repo / "crates/sta_mission/src/api"
        self.a2 = self.api / "a2"
        self.a2.mkdir(parents=True)
        (self.api / "entity.rs").write_text(
            'add_method("remove"); add_field_method_get("position"); '
            'add_field_method_get("valid"); add_field_method_get("destroyed");\n',
            encoding="utf-8",
        )
        (self.api / "player.rs").write_text(
            'add_method("getRelation"); add_method("setRelation");\n',
            encoding="utf-8",
        )
        (self.api / "timer.rs").write_text('add_method("stop");\n', encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def register_a2(self, objects, registration=None):
        (self.a2 / "objects.rs").write_text(objects, encoding="utf-8")
        (self.a2 / "tests.rs").write_text('table.set("TestOnly", value);', encoding="utf-8")
        (self.a2 / "mod.rs").write_text(
            registration or 'objects::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;',
            encoding="utf-8",
        )

    def test_mmm_aliases_are_limited_to_the_expected_source_module(self):
        self.assertEqual(inventory.covered_for("RemoveObject", self.repo), "Entity:remove")
        self.assertEqual(inventory.covered_for("GetRelation", self.repo), "Player:getRelation")
        self.assertEqual(inventory.covered_for("Stop", self.repo), "")
        self.assertEqual(inventory.covered_for("AI_Team_Load_AIP", self.repo), "")

    def test_a2_requires_exact_signature_literal_registration_and_namespace(self):
        self.register_a2('table.set("IsAlive", fn);\ntable.set("GetLocation", fn);\n')

        self.assertEqual(
            inventory.a2_exact_for(
                "public: bool __thiscall ScriptInterfaceImp::IsAlive(int)", self.repo
            ),
            "A2.IsAlive",
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: struct Vector3 const & __thiscall ScriptInterfaceImp::GetLocation(int)",
                self.repo,
            ),
            "A2.GetLocation",
        )
        self.assertEqual(
            inventory.a2_exact_for("Vector3 const & GetLocation(char *, int)", self.repo), ""
        )
        self.assertEqual(inventory.a2_exact_for("void SetRealTeam(int, int)", self.repo), "")
        self.assertEqual(inventory.a2_exact_for("void Cloak(int)", self.repo), "")
        self.assertEqual(
            inventory.covered_for("GetLocation", self.repo, "Vector3 const & GetLocation(int)"),
            "Entity.position",
        )
        self.assertEqual(
            inventory.covered_for(
                "GetLocation", self.repo, "Vector3 const & GetLocation(char *, int)"
            ),
            "",
        )

        self.register_a2('table.set("IsAlive", fn);', registration='objects::install(lua, ctx, &a2)?;')
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "")

    def test_exact_bindings_can_come_from_multiple_registered_modules(self):
        self.register_a2(
            'table.set("IsAlive", fn);',
            registration=(
                'objects::install(lua, ctx, &a2)?; '
                'subsystems::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "subsystems.rs").write_text(
            'table.set("DisableEngines", fn);\n'
            'table.set("DisableWeapons", fn);\n',
            encoding="utf-8",
        )
        self.assertEqual(
            inventory.a2_exact_for("void DisableEngines(int, bool)", self.repo),
            "A2.DisableEngines",
        )
        self.assertEqual(
            inventory.a2_exact_for("void DisableWeapons(int, bool)", self.repo),
            "A2.DisableWeapons",
        )
        self.assertEqual(inventory.a2_exact_for("void DisableEngines(int)", self.repo), "")
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "A2.IsAlive")
        self.register_a2('table.set("IsAlive", fn);')
        self.assertEqual(inventory.a2_exact_for("void DisableEngines(int, bool)", self.repo), "")

    def test_registered_query_signatures_require_exact_return_and_argument_types(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "queries.rs").write_text(
            'table.set("GetCurrentHealth", fn);\n'
            'table.set("GetMaxHealth", fn);\n'
            'table.set("IsOdf", fn);\n',
            encoding="utf-8",
        )

        self.assertEqual(
            inventory.a2_exact_for("float GetCurrentHealth(int)", self.repo),
            "A2.GetCurrentHealth",
        )
        self.assertEqual(
            inventory.a2_exact_for("float GetMaxHealth(int)", self.repo),
            "A2.GetMaxHealth",
        )
        self.assertEqual(
            inventory.a2_exact_for("bool IsOdf(int, char*)", self.repo), "A2.IsOdf"
        )
        self.assertEqual(inventory.a2_exact_for("int GetCurrentHealth(int)", self.repo), "")
        self.assertEqual(inventory.a2_exact_for("float GetMaxHealth(int, float)", self.repo), "")
        self.assertEqual(inventory.a2_exact_for("bool IsOdf(int, int)", self.repo), "")

    def test_distance_coverage_keeps_reference_arguments_and_overloads_distinct(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "queries.rs").write_text(
            'table.set("GetDistance", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: float __thiscall ScriptInterfaceImp::GetDistance(int &, int &)",
                self.repo,
            ),
            "A2.GetDistance",
        )
        for signature in [
            "int GetDistance(int&, int&)",
            "float GetDistance(int, int)",
            "float GetDistance(int&, char*, int)",
            "float GetDistance(int&, int&, int)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_test_files_do_not_count_as_a2_bindings(self):
        self.register_a2("", registration='objects::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;')
        (self.a2 / "tests.rs").write_text('table.set("IsAlive", fn);', encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "")

    def test_unregistered_production_modules_do_not_count(self):
        self.register_a2("")
        (self.a2 / "unregistered.rs").write_text('table.set("IsAlive", fn);', encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "")


if __name__ == "__main__":
    unittest.main()
