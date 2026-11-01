package dev.morizen.compat;

import java.nio.file.Path;
import java.util.Comparator;
import net.fabricmc.loader.api.ModContainer;
import net.fabricmc.loader.impl.FabricLoaderImpl;
import net.fabricmc.loader.impl.game.minecraft.Hooks;

/** Runs Fabric's real initializer adapter inside Knot's target class loader. */
public final class Bootstrap {
    private static ClassLoader targetLoader;

    private Bootstrap() {}

    public static void start(Path gameDirectory) {
        ClassLoader ownLoader = Bootstrap.class.getClassLoader();
        if (ownLoader == ClassLoader.getSystemClassLoader()) {
            throw new IllegalStateException("Morizen bootstrap was loaded outside Knot");
        }
        targetLoader = ownLoader;
        Thread thread = Thread.currentThread();
        thread.setName("Java mods");
        thread.setContextClassLoader(ownLoader);

        FabricLoaderImpl loader = FabricLoaderImpl.INSTANCE;
        String provider = loader.getGameProvider().getClass().getName();
        if (!provider.equals("dev.morizen.compat.provider.MorizenGameProvider")) {
            throw new IllegalStateException("Unexpected Fabric game provider " + provider);
        }
        System.out.println("morizen-java: loader=" + FabricLoaderImpl.VERSION);
        loader.getAllMods().stream()
                .sorted(Comparator.comparing(container -> container.getMetadata().getId()))
                .map(Bootstrap::describe)
                .forEach(description -> System.out.println("morizen-java: mod=" + description));

        // Fabric checks that the game instance itself came from Knot. A plain
        // java.lang.Object has the bootstrap class loader (null), so use this
        // target-loaded marker object as Morizen's logical game instance.
        Hooks.startClient(gameDirectory.toFile(), new MorizenGameInstance());
    }

    private static String describe(ModContainer container) {
        return container.getMetadata().getId() + "@"
                + container.getMetadata().getVersion().getFriendlyString();
    }

    public static void stop() {
        System.out.println("morizen-java: bootstrap stop");
        targetLoader = null;
        Thread.currentThread().setContextClassLoader(ClassLoader.getSystemClassLoader());
    }

    private static final class MorizenGameInstance {}
}
