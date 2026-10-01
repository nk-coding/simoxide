package headless.simucom;

import java.nio.file.Path;

/** Additional textual patches of generated sources (API changes beyond package moves). */
final class SourcePatches {
    private SourcePatches() {
    }

    /**
     * In 5.2.2, AbstractMain got two abstract methods that the templates do not implement (the IDE implementation is
     * AbstractMainUi in de.uka.ipd.sdq.simucomframework.ui, which is not in the product and would open a dialog).
     * We add headless implementations: issues are printed, the engine is the preferred one (as in AbstractMainUi).
     */
    static final String CONTROL_METHODS = "\n"
            + "\t@Override\n"
            + "\tprotected void handleModelIssues(java.util.List<de.uka.ipd.sdq.errorhandling.core.SeverityAndIssue> issues) {\n"
            + "\t\tfor (de.uka.ipd.sdq.errorhandling.core.SeverityAndIssue i : issues)\n"
            + "\t\t\tSystem.out.println(\"[simucom] model issue: \" + i.getError() + \" \" + i.getMessage());\n"
            + "\t}\n"
            + "\t@Override\n"
            + "\tprotected de.uka.ipd.sdq.simulation.abstractsimengine.ISimEngineFactory getSimulationEngine() {\n"
            + "\t\treturn de.uka.ipd.sdq.simulation.preferences.SimulationPreferencesHelper.getPreferredSimulationEngine();\n"
            + "\t}\n";

    static String apply(Path file, String text) {
        if (file.getFileName().toString().equals("SimuComControl.java")
                && text.contains("extends de.uka.ipd.sdq.simucomframework.AbstractMain")
                && !text.contains("handleModelIssues")) {
            int i = text.indexOf("extends de.uka.ipd.sdq.simucomframework.AbstractMain");
            int brace = text.indexOf('{', i);
            text = text.substring(0, brace + 1) + CONTROL_METHODS + text.substring(brace + 1);
        }
        return text;
    }

    /** Extra bundles the generated MANIFEST.MF must require. */
    static final String[] EXTRA_BUNDLES = { "de.uka.ipd.sdq.errorhandling.core" };
}
