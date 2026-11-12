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

    def test_station_requires_exact_signature_and_registered_query_binding(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        query = self.a2 / "queries.rs"
        query.write_text('table.set("IsStation", fn);\n', encoding="utf-8")
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual bool __thiscall ScriptInterfaceImp::IsStation(int)", self.repo
            ),
            "A2.IsStation",
        )
        for signature in (
            "int IsStation(int)",
            "bool IsStation()",
            "bool IsStation(int, bool)",
            "bool IsStation(unsigned int)",
            "bool IsStation(int&)",
        ):
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)
        query.write_text("", encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("bool IsStation(int)", self.repo), "")

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

    def test_race_coverage_keeps_pointer_qualifiers_distinct(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "queries.rs").write_text(
            'table.set("IsRace", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: bool __thiscall ScriptInterfaceImp::IsRace(int, char * const)",
                self.repo,
            ),
            "A2.IsRace",
        )
        for signature in [
            "bool IsRace(int, char*)",
            "bool IsRace(int, char const*)",
            "int IsRace(int, char*const)",
            "bool IsRace(int, char*const, int)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_visibility_coverage_requires_both_integer_arguments(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "queries.rs").write_text(
            'table.set("IsVisibleToTeam", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: bool __thiscall ScriptInterfaceImp::IsVisibleToTeam(int, int)",
                self.repo,
            ),
            "A2.IsVisibleToTeam",
        )
        for signature in [
            "int IsVisibleToTeam(int, int)",
            "bool IsVisibleToTeam(int)",
            "bool IsVisibleToTeam(int, float)",
            "bool IsVisibleToTeam(unsigned int, int)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_original_race_coverage_keeps_return_pointer_constness(self):
        self.register_a2(
            "",
            registration=(
                'queries::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "queries.rs").write_text(
            'table.set("GetOriginalRace", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual char const * __thiscall ScriptInterfaceImp::GetOriginalRace(int)",
                self.repo,
            ),
            "A2.GetOriginalRace",
        )
        for signature in [
            "char* GetOriginalRace(int)",
            "char*const GetOriginalRace(int)",
            "char const* GetOriginalRace(int&)",
            "char const* GetOriginalRace(int, int)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_objective_visibility_requires_exact_void_boolean_call_shape(self):
        self.register_a2(
            "",
            registration=(
                'objectives::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "objectives.rs").write_text(
            'table.set("ObjectivesDisplay_Set_Visibility", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual void __thiscall ScriptInterfaceImp::ObjectivesDisplay_Set_Visibility(bool)",
                self.repo,
            ),
            "A2.ObjectivesDisplay_Set_Visibility",
        )
        for signature in [
            "bool ObjectivesDisplay_Set_Visibility(bool)",
            "void ObjectivesDisplay_Set_Visibility(int)",
            "void ObjectivesDisplay_Set_Visibility(bool&)",
            "void ObjectivesDisplay_Set_Visibility(bool, bool)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_objective_completion_requires_exact_registered_void_int_bool_shape(self):
        self.register_a2(
            "",
            registration=(
                'objectives::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "objectives.rs").write_text(
            'table.set("ObjectivesDisplaySetObjectiveComplete", fn);\n', encoding="utf-8"
        )
        exact = "void ObjectivesDisplaySetObjectiveComplete(int, bool)"
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual void __thiscall ScriptInterfaceImp::ObjectivesDisplaySetObjectiveComplete(int, bool)",
                self.repo,
            ),
            "A2.ObjectivesDisplaySetObjectiveComplete",
        )
        for signature in [
            "bool ObjectivesDisplaySetObjectiveComplete(int, bool)",
            "void ObjectivesDisplaySetObjectiveComplete(bool, int)",
            "void ObjectivesDisplaySetObjectiveComplete(unsigned int, bool)",
            "void ObjectivesDisplaySetObjectiveComplete(int&, bool)",
            "void ObjectivesDisplaySetObjectiveComplete(int, int)",
            "void ObjectivesDisplaySetObjectiveComplete(int, bool, bool)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)
        (self.a2 / "objectives.rs").write_text("", encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for(exact, self.repo), "")
        (self.a2 / "objectives.rs").write_text(
            'table.set("ObjectivesDisplaySetObjectiveComplete", fn);\n', encoding="utf-8"
        )
        (self.a2 / "mod.rs").write_text(
            'objects::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;',
            encoding="utf-8",
        )
        self.assertEqual(inventory.a2_exact_for(exact, self.repo), "")

    def test_shared_control_and_objective_file_signatures_require_installed_exact_shapes(self):
        cases = [
            ("controls", "SetForceOntoMap", "void SetForceOntoMap(int, bool)", [
                "bool SetForceOntoMap(int, bool)", "void SetForceOntoMap(int, int)",
                "void SetForceOntoMap(int&, bool)", "void SetForceOntoMap(int)",
            ]),
            ("controls", "SetRealTeam", "void SetRealTeam(int, int)", [
                "int SetRealTeam(int, int)", "void SetRealTeam(int, unsigned int)",
                "void SetRealTeam(int&, int)", "void SetRealTeam(int, int, bool)",
            ]),
            ("objectives", "ObjectivesDisplay_Set_Text_From_File", "void ObjectivesDisplay_Set_Text_From_File(char*, bool)", [
                "bool ObjectivesDisplay_Set_Text_From_File(char*, bool)",
                "void ObjectivesDisplay_Set_Text_From_File(char const*, bool)",
                "void ObjectivesDisplay_Set_Text_From_File(char*, int)",
                "void ObjectivesDisplay_Set_Text_From_File(char*)",
            ]),
        ]
        for module, name, exact, wrong in cases:
            with self.subTest(name=name):
                registration = f'{module}::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;'
                self.register_a2("", registration=registration)
                path = self.a2 / f"{module}.rs"
                path.write_text(f'table.set("{name}", fn);\n', encoding="utf-8")
                self.assertEqual(inventory.a2_exact_for(exact, self.repo), f"A2.{name}")
                for signature in wrong:
                    self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)
                path.write_text("", encoding="utf-8")
                self.assertEqual(inventory.a2_exact_for(exact, self.repo), "")
                path.write_text(f'table.set("{name}", fn);\n', encoding="utf-8")
                (self.a2 / "mod.rs").write_text('lua.globals().raw_set("A2", a2)?;', encoding="utf-8")
                self.assertEqual(inventory.a2_exact_for(exact, self.repo), "")

    def test_perceived_team_signatures_require_exact_registered_control_shapes(self):
        self.register_a2(
            "",
            registration=(
                'controls::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "controls.rs").write_text(
            'table.set("SetPerceivedTeam", fn);\n'
            'table.set("GetPerceivedTeam", fn);\n',
            encoding="utf-8",
        )

        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual void __thiscall ScriptInterfaceImp::SetPerceivedTeam(int, int)",
                self.repo,
            ),
            "A2.SetPerceivedTeam",
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual int __thiscall ScriptInterfaceImp::GetPerceivedTeam(int)",
                self.repo,
            ),
            "A2.GetPerceivedTeam",
        )
        for signature in (
            "int SetPerceivedTeam(int, int)",
            "void SetPerceivedTeam(int)",
            "void SetPerceivedTeam(int, int&)",
            "void SetPerceivedTeam(int&, int)",
            "void SetPerceivedTeam(int, bool)",
            "void GetPerceivedTeam(int)",
            "bool GetPerceivedTeam(int)",
            "int GetPerceivedTeam()",
            "int GetPerceivedTeam(int&)",
            "int GetPerceivedTeam(int, int)",
        ):
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

        (self.a2 / "controls.rs").write_text('table.set("SetPerceivedTeam", fn);\n', encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("int GetPerceivedTeam(int)", self.repo), "")

    def test_presentation_request_signatures_need_registered_literals_and_exact_shapes(self):
        self.register_a2(
            "",
            registration=(
                'native_camera::install(lua, ctx, &a2)?; '
                'native_presentation::install(lua, ctx, &a2)?; '
                'lua.globals().raw_set("A2", a2)?;'
            ),
        )
        (self.a2 / "native_camera.rs").write_text(
            'table.set("CineractiveShot", fn);\n', encoding="utf-8"
        )
        (self.a2 / "native_presentation.rs").write_text(
            'table.set("MinimapMessage", fn);\ntable.set("GridVisible", fn);\n', encoding="utf-8"
        )
        for sig, name in (
            ("void CineractiveShot(char *, float, int)", "A2.CineractiveShot"),
            ("void CineractiveShot(int, int)", "A2.CineractiveShot"),
            ("void MinimapMessage(int, int, float)", "A2.MinimapMessage"),
            ("void MinimapMessage(char *, int, float, int)", "A2.MinimapMessage"),
            ("void GridVisible(bool)", "A2.GridVisible"),
        ):
            self.assertEqual(inventory.a2_exact_for(sig, self.repo), name, sig)
        for sig in (
            "void MinimapMessage(int, int)",
            "void MinimapMessage(char *, int, float)",
            "int MinimapMessage(int, int, float)",
            "void GridVisible(int)",
            "void CineractiveShot(char *, int)",
            # Module registered, but no literal table.set of this name: not counted.
            "void CenterCamera(int)",
            "void AddTransporterEffect(int)",
        ):
            self.assertEqual(inventory.a2_exact_for(sig, self.repo), "", sig)

    def test_current_health_writer_requires_exact_void_integer_float_call_shape(self):
        self.register_a2(
            'table.set("SetCurrentHealth", fn);\n',
            registration='objects::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;',
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual void __thiscall ScriptInterfaceImp::SetCurrentHealth(int, float)",
                self.repo,
            ),
            "A2.SetCurrentHealth",
        )
        for signature in [
            "float SetCurrentHealth(int, float)",
            "void SetCurrentHealth(int)",
            "void SetCurrentHealth(int, int)",
            "void SetCurrentHealth(int&, float)",
            "void SetCurrentHealth(int, float&)",
            "void SetCurrentHealth(int, float, bool)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_random_unsigned_requires_exact_unsigned_return_and_bound(self):
        self.register_a2(
            "",
            registration='random::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;',
        )
        (self.a2 / "random.rs").write_text(
            'table.set("RandomGetUnsigned", fn);\n', encoding="utf-8"
        )
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual unsigned int __thiscall ScriptInterfaceImp::RandomGetUnsigned(unsigned int)",
                self.repo,
            ),
            "A2.RandomGetUnsigned",
        )
        for signature in [
            "int RandomGetUnsigned(unsigned int)",
            "unsigned int RandomGetUnsigned(int)",
            "unsigned int RandomGetUnsigned(unsigned int&)",
            "unsigned int RandomGetUnsigned()",
            "unsigned int RandomGetUnsigned(unsigned int, unsigned int)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_special_energy_writer_requires_exact_void_integer_float_call_shape(self):
        self.register_a2('table.set("SetSpecialEnergy", fn);\n')
        self.assertEqual(
            inventory.a2_exact_for(
                "public: virtual void __thiscall ScriptInterfaceImp::SetSpecialEnergy(int, float)",
                self.repo,
            ),
            "A2.SetSpecialEnergy",
        )
        for signature in [
            "float SetSpecialEnergy(int, float)",
            "void SetSpecialEnergy(int)",
            "void SetSpecialEnergy(int, int)",
            "void SetSpecialEnergy(int&, float)",
            "void SetSpecialEnergy(int, float&)",
            "void SetSpecialEnergy(int, float, bool)",
        ]:
            self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)

    def test_existence_queries_require_each_exact_registered_call_shape(self):
        self.register_a2('table.set("Exists", fn);\ntable.set("DoesNotExist", fn);\n')
        for name in ("Exists", "DoesNotExist"):
            self.assertEqual(
                inventory.a2_exact_for(
                    f"public: virtual bool __thiscall ScriptInterfaceImp::{name}(int)",
                    self.repo,
                ),
                f"A2.{name}",
            )
            for signature in (
                f"void {name}(int)", f"int {name}(int)", f"bool {name}(float)",
                f"bool {name}(int&)", f"bool {name}(char*)", f"bool {name}()",
                f"bool {name}(int, int)",
            ):
                self.assertEqual(inventory.a2_exact_for(signature, self.repo), "", signature)
        self.register_a2('table.set("Exists", fn);')
        self.assertEqual(inventory.a2_exact_for("bool DoesNotExist(int)", self.repo), "")
        self.register_a2('table.set("DoesNotExist", fn);')
        self.assertEqual(inventory.a2_exact_for("bool Exists(int)", self.repo), "")

    def test_test_files_do_not_count_as_a2_bindings(self):
        self.register_a2("", registration='objects::install(lua, ctx, &a2)?; lua.globals().raw_set("A2", a2)?;')
        (self.a2 / "tests.rs").write_text('table.set("IsAlive", fn);', encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "")

    def test_unregistered_production_modules_do_not_count(self):
        self.register_a2("")
        (self.a2 / "unregistered.rs").write_text('table.set("IsAlive", fn);', encoding="utf-8")
        self.assertEqual(inventory.a2_exact_for("bool IsAlive(int)", self.repo), "")

    def register_a1(self, module, source, registration=None):
        a1 = self.api / "a1"
        a1.mkdir(exist_ok=True)
        (a1 / f"{module}.rs").write_text(source, encoding="utf-8")
        (a1 / "tests.rs").write_text('table.set("TestOnly", value);', encoding="utf-8")
        (a1 / "mod.rs").write_text(
            registration
            or f'{module}::install(lua, &env, &table)?; lua.globals().raw_set("A1", table)?;',
            encoding="utf-8",
        )

    def test_a1_requires_exact_import_signature_literal_registration_and_namespace(self):
        self.register_a1("presentation", 'table.set("StopMovie", f);\ntable.set("PlayBridgeMovie", f);\n')
        self.assertEqual(inventory.a1_exact_for("void StopMovie(void)", self.repo), "A1.StopMovie")
        self.assertEqual(
            inventory.a1_exact_for("void PlayBridgeMovie(char *, int)", self.repo), "A1.PlayBridgeMovie"
        )
        # A different overload or return type is not the imported function.
        self.assertEqual(inventory.a1_exact_for("void PlayBridgeMovie(char *)", self.repo), "")
        self.assertEqual(inventory.a1_exact_for("bool StopMovie(void)", self.repo), "")
        # A name the source does not bind is not callable even if the table lists it.
        self.assertEqual(inventory.a1_exact_for("bool IsMoviePlaying(void)", self.repo), "")

    def test_a1_enum_argument_signatures_are_matched_whole(self):
        self.register_a1("presentation", 'table.set("SetBridgeSceneRace", f);\n')
        self.assertEqual(
            inventory.a1_exact_for("void SetBridgeSceneRace(BridgeDisplay::BridgeType)", self.repo),
            "A1.SetBridgeSceneRace",
        )

    def test_a1_ignores_unregistered_modules_test_files_and_other_namespaces(self):
        self.register_a1("presentation", "")
        (self.api / "a1" / "unregistered.rs").write_text('table.set("StopMovie", f);', encoding="utf-8")
        self.assertEqual(inventory.a1_exact_for("void StopMovie(void)", self.repo), "")
        self.register_a1(
            "presentation",
            'table.set("StopMovie", f);',
            registration='presentation::install(lua, &env, &table)?; lua.globals().raw_set("B1", table)?;',
        )
        self.assertEqual(inventory.a1_exact_for("void StopMovie(void)", self.repo), "")

    def test_a1_rule_and_group_machinery_is_script_state_plumbing(self):
        for sig in (
            "rule * IIIE::create_rule(int (*)(void), void (*)(void), char *)",
            "group::group(char *)",
            "void Add_Team_To_Group(int, group &)",
            "hot_float strength_team_1",
            "int Load_Rules(char *)",
        ):
            self.assertEqual(inventory.area(sig), "Script state and save/load", sig)
        self.assertNotEqual(inventory.area("void PlayBridgeMovie(char *, int)"), "Script state and save/load")

    def test_every_a1_signature_names_a_distinct_overload(self):
        keys = [inventory.normalized_signature(sig) for sig in inventory.A1_SIGNATURES]
        self.assertEqual(len(keys), len(set(keys)))
        for sig, lua_name in inventory.A1_SIGNATURES.items():
            self.assertTrue(lua_name.startswith("A1."), sig)


if __name__ == "__main__":
    unittest.main()
