package dev.morizen.compat;

import dev.morizen.compat.provider.MorizenGameProvider;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Enumeration;
import java.util.Set;
import java.util.TreeSet;
import net.fabricmc.loader.impl.game.GameProvider;

/** Locks the provider implementation to Fabric Loader 0.19.5's internal interface. */
public final class LoaderContractTest {
    private static final Set<String> EXPECTED = Set.of(
            "canOpenErrorGui()->boolean",
            "displayCrash(java.lang.Throwable,java.lang.String)->boolean",
            "getArguments()->net.fabricmc.loader.impl.util.Arguments",
            "getBuiltinMods()->java.util.Collection",
            "getBuiltinTransforms(java.lang.String)->java.util.Set",
            "getDefaultModDistributionNamespace(java.lang.String)->java.lang.String",
            "getEntrypoint()->java.lang.String",
            "getEntrypointTransformer()->net.fabricmc.loader.impl.game.patch.GameTransformer",
            "getGameId()->java.lang.String",
            "getGameName()->java.lang.String",
            "getLaunchArguments(boolean)->java.lang.String[]",
            "getLaunchDirectory()->java.nio.file.Path",
            "getNormalizedGameVersion()->java.lang.String",
            "getRawGameVersion()->java.lang.String",
            "getRuntimeNamespace(java.lang.String)->java.lang.String",
            "hasAwtSupport()->boolean",
            "initialize(net.fabricmc.loader.impl.launch.FabricLauncher)->void",
            "isEnabled()->boolean",
            "launch(java.lang.ClassLoader)->void",
            "locateGame(net.fabricmc.loader.impl.launch.FabricLauncher,java.lang.String[])->boolean",
            "requiresUrlClassLoader()->boolean",
            "unlockClassPath(net.fabricmc.loader.impl.launch.FabricLauncher)->void");

    private LoaderContractTest() {}

    public static void main(String[] arguments) throws Exception {
        Set<String> actual = new TreeSet<>();
        for (Method method : GameProvider.class.getDeclaredMethods()) {
            if (Modifier.isPublic(method.getModifiers())) {
                actual.add(signature(method));
            }
        }
        if (!actual.equals(EXPECTED)) {
            Set<String> missing = new TreeSet<>(EXPECTED);
            missing.removeAll(actual);
            Set<String> added = new TreeSet<>(actual);
            added.removeAll(EXPECTED);
            throw new AssertionError("GameProvider contract changed; missing=" + missing + ", added=" + added);
        }
        if (!GameProvider.class.isAssignableFrom(MorizenGameProvider.class)) {
            throw new AssertionError("MorizenGameProvider no longer implements GameProvider");
        }
        MorizenGameProvider provider = new MorizenGameProvider();
        if (!provider.getGameId().equals("minecraft")
                || !provider.getRuntimeNamespace("minecraft").equals("official")
                || !provider.getDefaultModDistributionNamespace("minecraft").equals("official")) {
            throw new AssertionError("Morizen provider identity/namespace changed");
        }
        assertServiceRegistration();
        System.out.println("morizen-java: loader contract=0.19.5 methods=" + actual.size());
    }

    private static String signature(Method method) {
        String parameters = Arrays.stream(method.getParameterTypes())
                .map(Class::getTypeName)
                .reduce((left, right) -> left + "," + right)
                .orElse("");
        return method.getName() + "(" + parameters + ")->" + method.getReturnType().getTypeName();
    }

    private static void assertServiceRegistration() throws Exception {
        String service = "META-INF/services/" + GameProvider.class.getName();
        Enumeration<URL> resources = LoaderContractTest.class.getClassLoader().getResources(service);
        while (resources.hasMoreElements()) {
            String contents = new String(resources.nextElement().openStream().readAllBytes(), StandardCharsets.UTF_8);
            if (contents.lines().anyMatch(MorizenGameProvider.class.getName()::equals)) {
                return;
            }
        }
        throw new AssertionError("Missing Morizen GameProvider service registration");
    }
}
