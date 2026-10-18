from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class OnboardingEncodingTest(unittest.TestCase):
    def test_starter_copy_survives_production_build(self):
        html = (ROOT / "dist" / "index.html").read_text(encoding="utf-8")
        bundle = "\n".join(
            path.read_text(encoding="utf-8")
            for path in (ROOT / "dist" / "assets").glob("*.js")
        )
        self.assertIn('<meta charset="UTF-8"', html)
        for phrase in (
            "Выберите первую модель",
            "Загружаем сведения о модели",
            "Пропустить и начать работу",
            "Пример изображения WAI-illustrious-SDXL",
        ):
            self.assertIn(phrase, bundle)
        self.assertNotIn("\ufffd", bundle)
        self.assertNotIn("????", bundle)


if __name__ == "__main__":
    unittest.main()
